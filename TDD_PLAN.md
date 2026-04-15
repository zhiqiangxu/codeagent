# CodeForge TDD 开发方案

## Context

项目当前是绿地状态（仅有 ARCHITECTURE.md），需要从 Phase 1 MVP 到 Phase 4 IDE+部署全链路落地。本方案以 **7 个核心 trait** 为骨架，采用严格的 **Red-Green-Refactor** TDD 方法论，覆盖全部 4 个 Phase、18 个迭代。

核心策略：**trait 一步到位，实现分阶段演进**。Phase 1 对尚未实现的 trait 提供 Noop 占位实现，后续 Phase 逐步替换为真实实现——每次替换都先写测试。

---

## 核心 Trait 清单

迭代 0 一次性定义全部 7 个 trait，它们是整个 TDD 方案的测试锚点。

```rust
// 1. ModelProvider — 谁来回答（模型调用）
// 职责：封装不同 LLM 厂商（Anthropic / OpenAI / Gemini）的调用细节，
//       为上层提供统一的流式对话接口。ModelRouter 持有多个 ModelProvider，
//       根据 capabilities() 动态选择最合适的模型。
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// 向 LLM 发起流式对话请求，以 chunk 方式逐步返回模型生成的内容。
    ///
    /// - `req`: 包含 messages、temperature、tools 等参数的完整请求体。
    /// - 返回: `StreamResponse`，可通过 `.next()` 逐块读取 delta 文本或 tool_use 事件。
    ///
    /// 场景：Agent 主循环每轮都会调用此方法，将编排好的上下文发送给 LLM，
    ///       并实时将回复流式输出到终端（打字机效果）。
    async fn chat_stream(&self, req: ChatRequest) -> Result<StreamResponse>;

    /// 返回当前模型的能力描述（最大上下文长度、是否支持 tool_use / vision / JSON mode 等）。
    ///
    /// 场景：ModelRouter 在多模型场景下，根据任务需求（如需要 vision）过滤出
    ///       具备相应能力的 provider；ContextEngine 也用 max_tokens 来决定
    ///       token 预算上限。
    fn capabilities(&self) -> ModelCapabilities;

    /// 计算给定消息列表消耗的 token 数（近似值，用于预算控制）。
    ///
    /// - `messages`: 待计算的消息列表。
    /// - 返回: 估算的 token 总数。
    ///
    /// 场景：ContextEngine.assemble() 在拼装上下文时，反复调用此方法检查
    ///       当前消息总 token 是否超出预算，超出则触发 compact() 压缩。
    ///       不同模型的 tokenizer 不同（cl100k / sentencepiece），
    ///       所以此方法由各 provider 自行实现。
    fn token_counter(&self, messages: &[Message]) -> usize;
}

// 2. ContextEngine — 看到什么（上下文编排）
// 职责：决定每次发给 LLM 的消息列表内容——在有限的 token 预算内，
//       选择最有价值的信息（系统提示 + 历史对话 + 记忆检索结果）组装成最终上下文。
//       Phase 1 做简单截断，Phase 2 集成 MemoryRetriever 补充相关记忆。
#[async_trait]
pub trait ContextEngine: Send + Sync {
    /// 在给定 token 预算内，将消息列表编排成最终要发给 LLM 的上下文。
    ///
    /// - `messages`: 原始消息列表（系统提示 + 用户/助手对话历史）。
    /// - `budget`: token 上限，编排结果不得超过此值。
    /// - 返回: 编排后的消息列表，可能包含从 MemoryRetriever 检索到的额外上下文。
    ///
    /// 场景：每次调用 LLM 前的必经步骤。Phase 1 简单地从尾部截断超出部分；
    ///       Phase 2 会先调用 MemoryRetriever.retrieve() 检索相关记忆片段，
    ///       插入到上下文中，再按优先级截断。
    async fn assemble(&self, messages: &[Message], budget: TokenBudget) -> Vec<Message>;

    /// 当历史消息总 token 超出预算时，对旧消息进行压缩/精简。
    ///
    /// - `messages`: 需要压缩的消息列表（通常是较早的历史对话）。
    /// - `target`: 压缩后的目标 token 上限。
    /// - 返回: 压缩后的消息列表（可能将多轮对话合并为一条摘要消息）。
    ///
    /// 场景：长对话中历史不断累积，assemble() 检测到超出预算后调用此方法。
    ///       内部委托 CompactionProvider.summarize() 生成摘要，
    ///       用一条摘要消息替换多条旧消息，腾出 token 空间。
    async fn compact(&self, messages: &[Message], target: TokenBudget) -> Vec<Message>;
}

// 3. CompactionProvider — 怎么压缩（历史压缩策略）
// 职责：提供具体的消息压缩/摘要算法。ContextEngine.compact() 委托此 trait 执行实际压缩。
//       Phase 1 用 NoopCompaction（不压缩，原样返回），
//       Phase 2 替换为 LlmCompaction（调用 LLM 生成对话摘要）。
#[async_trait]
pub trait CompactionProvider: Send + Sync {
    /// 将一组消息摘要为一段简短的文本总结。
    ///
    /// - `messages`: 需要被摘要的消息列表。
    /// - 返回: 摘要文本字符串。
    ///
    /// 场景：ContextEngine.compact() 内部调用。NoopCompaction 直接拼接原文返回；
    ///       LlmCompaction 构造一个"请总结以下对话"的 prompt 发给 LLM，
    ///       用返回的摘要替换原始消息，通常可将 token 数压缩到原来的 1/5~1/10。
    async fn summarize(&self, messages: &[Message]) -> Result<String>;
}

// 4. MemoryRetriever — 记住什么（记忆检索）
// 职责：从持久化记忆库（SQLite + 向量索引）中检索与当前查询相关的代码片段、
//       项目文档等，供 ContextEngine 注入上下文。
//       Phase 1 用 ForgemdRetriever（全量加载 FORGE.md），
//       Phase 2 用 HybridRetriever（向量相似度 + FTS5 全文检索混合排序）。
#[async_trait]
pub trait MemoryRetriever: Send + Sync {
    /// 根据查询语句检索最相关的记忆片段。
    ///
    /// - `query`: 用户的自然语言查询或从对话中提取的关键词。
    /// - `opts`: 检索选项（top_k 返回条数、score 阈值、过滤条件等）。
    /// - 返回: 按相关度排序的 MemoryChunk 列表，每个 chunk 包含文本内容、来源路径、得分。
    ///
    /// 场景：ContextEngine.assemble() 在编排上下文时调用，将检索到的记忆片段
    ///       作为 system 消息的一部分注入 LLM prompt，帮助模型理解项目背景。
    ///       ForgemdRetriever 忽略 query 直接返回全文；
    ///       HybridRetriever 先将 query 向量化，再同时做向量 ANN 和 FTS5 检索，
    ///       用 RRF（Reciprocal Rank Fusion）合并排序。
    async fn retrieve(&self, query: &str, opts: RetrieveOptions) -> Vec<MemoryChunk>;

    /// 对指定文件建立索引（切片 → 向量化 → 存入 SQLite）。
    ///
    /// - `files`: 需要索引的文件路径列表。
    /// - 返回: 成功或错误。
    ///
    /// 场景：项目初始化时批量索引代码文件，或文件变更时增量更新索引。
    ///       内部流程：读取文件 → 按函数/段落切片 → 调用 EmbeddingProvider.embed()
    ///       向量化 → 写入 SQLite（向量列 + FTS5 全文索引）。
    ///       ForgemdRetriever 的此方法为 noop（FORGE.md 无需索引）。
    async fn index(&self, files: &[PathBuf]) -> Result<()>;
}

// 5. EmbeddingProvider — 怎么向量化（文本嵌入）
// 职责：将文本转换为高维向量表示，供 MemoryRetriever 做相似度检索。
//       Phase 1 用 NoopEmbedding（返回零向量占位），
//       Phase 2 替换为 OpenAI/Gemini Embedding 真实实现。
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// 将一批文本转换为向量表示（embedding）。
    ///
    /// - `texts`: 待向量化的文本列表（支持批量以减少 API 调用次数）。
    /// - 返回: 与输入一一对应的向量列表，每个向量为 `Vec<f32>`。
    ///
    /// 场景：
    ///   1. 建索引时：MemoryRetriever.index() 将代码片段批量向量化后存入 SQLite。
    ///   2. 检索时：MemoryRetriever.retrieve() 将用户查询向量化，
    ///      再与数据库中的向量做余弦相似度 / 内积计算，找出最相关的片段。
    ///   NoopEmbedding 返回全零向量（维度由 dimension() 决定），仅用于 Phase 1 占位。
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// 返回向量维度（如 OpenAI text-embedding-ada-002 为 1536 维，
    /// text-embedding-3-small 为 1536 维，Gemini embedding 为 768 维）。
    ///
    /// 场景：初始化 SQLite 向量表时，需要知道维度来定义列结构；
    ///       NoopEmbedding 也需要此值来生成正确长度的零向量。
    fn dimension(&self) -> usize;
}

// 6. RuntimePrompter — 问不问用户（权限交互）
// 职责：在 Agent 执行危险工具（如 bash、write_file）前，向用户请求授权。
//       PermissionGateway 内部持有 RuntimePrompter，根据权限规则决定是否需要弹窗。
//       Phase 1 用 TuiRuntimePrompter（终端 TUI 弹窗），
//       Phase 4 增加 LspRuntimePrompter（IDE 弹窗）和 GrpcRuntimePrompter（远程 RPC）。
#[async_trait]
pub trait RuntimePrompter: Send + Sync {
    /// 询问用户是否授权执行指定工具。
    ///
    /// - `tool_name`: 待执行的工具名称（如 "bash", "write_file"）。
    /// - `args`: 工具参数的 JSON 字符串（如 bash 的命令内容），展示给用户审核。
    /// - 返回: `PermissionDecision` 枚举——Allow（本次允许）、
    ///         AlwaysAllow（永久允许该工具）、Deny（拒绝）。
    ///
    /// 场景：Agent 循环中每次执行 tool_use 前，PermissionGateway 先检查
    ///       该工具是否已在白名单中；若不在，则调用此方法弹出交互式确认框，
    ///       用户可选择允许/拒绝/永久允许。TUI 实现用 crossterm 渲染终端弹窗，
    ///       LSP 实现通过 window/showMessageRequest 在 IDE 中弹窗。
    async fn ask(&self, tool_name: &str, args: &str) -> PermissionDecision;
}

// 7. Tool — 能做什么（工具扩展）
// 职责：统一抽象所有可被 Agent 调用的工具——内置工具（read/write/edit/bash/glob/grep）、
//       记忆工具（memory_search/memory_save）、MCP 动态工具、Skills 等。
//       ToolRegistry 持有 Vec<Box<dyn Tool>>，Agent 根据 LLM 返回的 tool_use.name
//       查找并调用对应工具。
#[async_trait]
pub trait Tool: Send + Sync {
    /// 返回工具的唯一标识名称（如 "read_file", "bash", "glob"）。
    ///
    /// 场景：ToolRegistry 注册时用作 key；Agent 收到 LLM 的 tool_use 响应后，
    ///       用此名称在 registry 中查找对应的 Tool 实例。
    ///       名称需与 LLM prompt 中声明的工具名保持一致。
    fn name(&self) -> &str;

    /// 返回工具的自然语言功能描述。
    ///
    /// 场景：组装 system prompt 的 tools 部分时，将每个工具的 description
    ///       告知 LLM，让模型理解各工具的用途和适用场景，以便正确选择工具。
    ///       描述应简明扼要，突出工具能做什么、何时该用。
    fn description(&self) -> &str;

    /// 返回工具参数的 JSON Schema 定义。
    ///
    /// 场景：作为 Anthropic API `tools[].input_schema` 或 OpenAI API
    ///       `tools[].function.parameters` 发送给 LLM，
    ///       LLM 据此生成符合格式的参数 JSON。
    ///       例如 bash 工具的 schema 定义 command(string, required) 和
    ///       timeout(integer, optional) 两个参数。
    fn schema(&self) -> serde_json::Value;

    /// 执行工具逻辑，传入 LLM 生成的参数，返回执行结果。
    ///
    /// - `args`: LLM 生成的工具调用参数（已解析为 serde_json::Value）。
    /// - 返回: `ToolOutput`，包含 stdout/stderr 文本或结构化数据，
    ///         会作为 tool_result 回传给 LLM 进行下一轮推理。
    ///
    /// 场景：Agent 主循环的核心步骤——LLM 返回 tool_use → 权限检查通过 →
    ///       调用此方法执行 → 结果封装为 tool_result 消息追加到对话历史 →
    ///       再次调用 LLM。内置 bash 工具在此方法中 spawn 子进程；
    ///       MCP 工具在此方法中通过 JSON-RPC 调用远程 MCP Server。
    async fn execute(&self, args: serde_json::Value) -> Result<ToolOutput>;
}
```

## Trait 实现演进表

| Trait | Phase 1 实现 | Phase 2 实现 | Phase 3 实现 | Phase 4 实现 |
|---|---|---|---|---|
| `ModelProvider` | Anthropic/OpenAI/Gemini Provider | — | — | — |
| `ContextEngine` | 简单截断（不调 LLM） | 集成 MemoryRetriever 补充上下文 | — | — |
| `CompactionProvider` | `NoopCompaction`（不压缩） | `LlmCompaction`（LLM 摘要） | — | — |
| `MemoryRetriever` | `ForgemdRetriever`（仅 FORGE.md 全量加载） | `HybridRetriever`（向量+FTS5） | — | — |
| `EmbeddingProvider` | `NoopEmbedding` | OpenAI/Gemini Embedding | — | — |
| `RuntimePrompter` | `TuiRuntimePrompter` | — | — | +`LspRuntimePrompter` +`GrpcRuntimePrompter` |
| `Tool` | 6 个内置工具（read/write/edit/bash/glob/grep） | +memory_search/memory_save | +MCP 动态工具 +Skills +agent_spawn | — |

"—" 表示该 Phase 无变化，沿用上一 Phase 的实现。

---

## 测试策略

每个迭代的测试分为两类：

### Unit Test（单元测试）
- **位置**: `crates/forge-xxx/src/*.rs` 中的 `#[cfg(test)] mod tests`
- **特点**: 测试单个 struct/fn 的内部逻辑，不跨 crate 边界
- **依赖**: 无外部依赖，或仅用 `mockall` mock 同 crate 的 trait
- **速度**: 毫秒级
- **命名**: `test_<struct>_<behavior>`

### Integration Test（集成测试）
- **位置**: `crates/forge-xxx/tests/*.rs`
- **特点**: 测试多个模块协作、跨 crate 调用、真实 I/O
- **依赖**: 用 `MockXxx` 或 `ScriptedXxx` 替代外部服务，用 `tempfile` 替代真实文件系统
- **速度**: 秒级，涉及真实 API 的标记 `#[ignore]`
- **命名**: `test_<场景>_<预期结果>`

### Mock 策略总表

| Trait | Mock 类型 | 用于哪些集成测试 |
|---|---|---|
| `ModelProvider` | `MockModelProvider`（mockall）+ `ScriptedModelProvider`（按剧本返回） | Router failover、Agent Loop、E2E |
| `ContextEngine` | `MockContextEngine`（mockall） | Agent Loop |
| `CompactionProvider` | `MockCompactionProvider`（mockall） | ContextEngine |
| `MemoryRetriever` | `MockMemoryRetriever`（mockall） | ContextEngine |
| `EmbeddingProvider` | `MockEmbeddingProvider`（mockall） | HybridRetriever |
| `RuntimePrompter` | `MockRuntimePrompter`（mockall）+ `AutoAllowPrompter` / `AutoDenyPrompter` | PermissionGateway、ToolEngine、E2E |
| `Tool` | `MockTool`（mockall） | ToolRegistry、ToolEngine |

---

## 依赖图（构建顺序）

```
Phase 1:
  forge-permissions  (叶子，无内部依赖)
  forge-model        (叶子，无内部依赖)
  forge-memory       (叶子，Phase 1 ForgemdRetriever → Phase 2 HybridRetriever)
  forge-tools        (依赖 forge-permissions)
  forge-mcp          (依赖 forge-tools，Phase 1 基础版 → Phase 3 完善)
  forge-core         (依赖 forge-model, forge-tools, forge-memory)
  forge-tui          (依赖 forge-core)
  forge-cli          (依赖 forge-core, forge-tui)

测试辅助:
  forge-test-utils   (共享 fixture/mock/helper，被所有 crate 的 dev-dependencies 引用)

Phase 4 新增:
  forge-lsp          (依赖 forge-core)
  forge-gateway      (依赖 forge-core)
```

---

# Phase 1: MVP

## 迭代 0: 项目骨架 + 7 个核心 Trait + Noop 实现 (1 天)

**目标**: Cargo workspace 骨架、7 个核心 trait 定义、Noop 占位实现、核心类型、测试基础设施

### 脚手架（非 TDD）
- 根 `Cargo.toml` workspace 配置，所有 crate 占位
- `[workspace.dependencies]` 统一版本
- `forge-test-utils` crate：共享 fixture/helper

### 核心类型（forge-core/src/types.rs）

```rust
pub enum Role { User, Assistant, System }
pub struct Message { pub role: Role, pub content: Content, pub tool_calls: Vec<ToolCall> }
pub enum Content { Text(String), ToolResult { tool_use_id: String, output: String } }
pub struct ToolCall { pub id: String, pub name: String, pub arguments: serde_json::Value }
pub struct ToolOutput { pub content: String, pub is_error: bool }
pub struct TokenBudget { pub max_tokens: usize, pub reserved: usize }
```

### 7 个核心 Trait 定义

全部定义在对应 crate 的 `lib.rs` 中（见上方"核心 Trait 清单"）。

### Noop 占位实现

```rust
pub struct NoopCompaction;       // summarize() → Ok("".into())
pub struct NoopRetriever;        // retrieve() → vec![], index() → Ok(())
pub struct NoopEmbedding;        // embed() → Ok(vec![]), dimension() → 0
```

**Unit Tests** (`forge-core/src/types.rs`):
- `test_message_serialization` — 构造 Message{role: User, content: Text("hi")} → serde_json::to_string → from_str → 字段相等
- `test_token_budget_available` — TokenBudget{max:4096, reserved:512}.available() == 3584
- `test_token_budget_zero_reserved` — TokenBudget{max:4096, reserved:0}.available() == 4096
- `test_tool_output_error_flag` — ToolOutput{content:"err", is_error:true}.is_error == true
- `test_tool_call_from_json` — 从 `{"id":"1","name":"read","arguments":{"path":"/a"}}` 反序列化 → ToolCall 字段正确

**Unit Tests** (`forge-core/src/noop.rs` 或各 crate):
- `test_noop_compaction_returns_empty` — NoopCompaction.summarize(&[msg]).await == Ok("")
- `test_noop_retriever_returns_empty` — NoopRetriever.retrieve("any", opts).await == vec![]
- `test_noop_retriever_index_ok` — NoopRetriever.index(&[path]).await == Ok(())
- `test_noop_embedding_returns_empty` — NoopEmbedding.embed(&["text"]).await == Ok(vec![])
- `test_noop_embedding_dimension_zero` — NoopEmbedding.dimension() == 0

---

## 迭代 1: RuntimePrompter — forge-permissions (2 天)

**测试目标**: `RuntimePrompter` trait + PermissionGateway

### 1.1 权限模型数据结构

**Unit Tests** (`forge-permissions/src/lib.rs`):
- `test_profile_readonly_denies_write` — Profile::ReadOnly.allows("write") == false, .allows("read") == true
- `test_profile_coding_allows_write` — Profile::Coding.allows("write") == true, .allows("bash") == true
- `test_profile_full_allows_all` — Profile::Full.allows(任意工具名) == true
- `test_rule_parse_glob_pattern` — Rule::parse("Bash(git *)") → Rule { tool: "Bash", pattern: "git *", action: AutoAllow }
- `test_rule_parse_deny_pattern` — Rule::parse("Bash(rm -rf *)") → action: AlwaysDeny
- `test_rule_parse_invalid` — Rule::parse("garbage") → Err
- `test_rule_match_positive` — Rule{pattern:"git *"}.matches("git status") == true
- `test_rule_match_negative` — Rule{pattern:"git *"}.matches("rm -rf /") == false
- `test_rules_ordering` — vec![Rule("*", Ask), Rule("git *", AutoAllow)] → "git status" 匹配更具体的 AutoAllow

**Green**: `enum Profile`, `struct Rule`, `enum Action`, `fn matches()`

### 1.2 PermissionGateway（消费 RuntimePrompter trait）

**Unit Tests** (`forge-permissions/src/gateway.rs`):
- `test_gateway_auto_allow_matching_rule` — rules 含 AutoAllow("git *"), check("Bash","git status") → Permit，不调 RuntimePrompter
- `test_gateway_always_deny_matching_rule` — rules 含 AlwaysDeny("rm *"), check("Bash","rm -rf /") → Deny，不调 RuntimePrompter
- `test_gateway_readonly_blocks_write_tool` — profile=ReadOnly, check("write", any) → Deny，不检查 rules

**Integration Tests** (`forge-permissions/tests/gateway.rs`):
- `test_gateway_no_rule_asks_runtime` — 无匹配规则 → 调用 MockRuntimePrompter.ask() → 验证 mock 被调用 1 次，参数正确
- `test_gateway_runtime_allow_generates_rule` — MockRuntimePrompter 返回 AlwaysAllow → 验证 rules 新增一条 AutoAllow 规则
- `test_gateway_rule_persistence_roundtrip` — 保存 rules 到 tempfile → 重新加载 → 逐条比较相等

**Mock**: `mockall` 生成 `MockRuntimePrompter`，控制 ask() 返回值并验证调用次数

---

## 迭代 2: ModelProvider — 类型 + Router (2 天)

**测试目标**: `ModelProvider` trait + ModelRouter

### 2.1 ModelProvider trait 周边类型

**Unit Tests** (`forge-model/src/types.rs`):
- `test_model_id_parse_anthropic` — ModelId::parse("claude-sonnet-4-20250514") → provider: Anthropic, name: "claude-sonnet-4-20250514"
- `test_model_id_parse_openai` — ModelId::parse("gpt-4o") → provider: OpenAI
- `test_model_id_parse_gemini` — ModelId::parse("gemini-2.0-flash") → provider: Gemini
- `test_model_id_parse_unknown` — ModelId::parse("unknown-model") → Err(UnknownProvider)
- `test_model_capabilities` — ModelCapabilities { streaming: true, tool_use: true, vision: false } 字段访问正确
- `test_chat_request_builder` — ChatRequest::builder().model("x").messages(vec![]).tools(vec![]).build() → 各字段正确
- `test_stream_event_delta` — StreamEvent::Delta { content: "hi" } 构造和 match 正确
- `test_stream_event_tool_call` — StreamEvent::ToolCall { id, name, arguments } 构造正确
- `test_stream_event_done` — StreamEvent::Done { usage: TokenUsage{input:10, output:5} }
- `test_token_usage_add` — TokenUsage{input:10,output:5} + TokenUsage{input:20,output:10} == {input:30,output:15}

### 2.2 ModelRouter（消费 ModelProvider trait）

**Integration Tests** (`forge-model/tests/router.rs`):
- `test_router_selects_correct_provider` — 注册 MockA(name="anthropic") + MockB(name="openai") → route("claude-xxx") → MockA 被调用
- `test_router_unknown_model_error` — route("not-exist") → Err(UnknownModel("not-exist"))
- `test_router_failover_on_error` — MockA.chat_stream 返回 Err(503) → 自动切到 MockB → MockB 被调用 → 返回成功
- `test_router_failover_exhausted` — MockA 返回 503, MockB 返回 503 → Err(AllProvidersFailed)，包含最后一个错误
- `test_router_no_retry_on_auth_error` — MockA 返回 Err(401) → 不 failover → 直接 Err(AuthError)

**Mock**: `MockModelProvider`（mockall），`.expect_chat_stream().returning(|_| ...)` 控制返回值

---

## 迭代 3: ModelProvider — 三个 Provider 实现 (2 天)

**测试目标**: `ModelProvider` trait 的三个真实实现

### 3.1 AnthropicProvider

**Unit Tests** (`forge-model/src/anthropic.rs`):
- `test_anthropic_format_request` — Message{role:User, content:Text("hi")} → JSON body 包含 `{"role":"user","content":"hi"}`
- `test_anthropic_format_system` — Message{role:System} → 放在 `system` 字段而非 `messages` 数组
- `test_anthropic_format_tool_definition` — ToolDef{name:"read",schema:{...}} → Anthropic `tools` 格式含 `input_schema`
- `test_anthropic_parse_stream_delta` — SSE `event:content_block_delta data:{"delta":{"text":"hi"}}` → StreamEvent::Delta{content:"hi"}
- `test_anthropic_parse_stream_tool_use` — SSE `content_block_start` type=tool_use → StreamEvent::ToolCall{id,name,arguments}
- `test_anthropic_parse_stream_done` — SSE `event:message_stop` → StreamEvent::Done{usage}
- `test_anthropic_error_429` — HTTP 429 → ModelError::RateLimited
- `test_anthropic_error_401` — HTTP 401 → ModelError::AuthError
- `test_anthropic_error_503` — HTTP 503 → ModelError::ServiceUnavailable（可重试）
- `test_anthropic_token_counter` — token_counter(&[msg]) → 粗略计数（字符/4）

**Integration Tests** (`forge-model/tests/anthropic.rs`, `#[ignore]`):
- `test_anthropic_real_stream` — 真实 API key → chat_stream("say hi") → 收到 Delta + Done 事件

### 3.2 OpenAIProvider

**Unit Tests** (`forge-model/src/openai.rs`，结构与 Anthropic 对称):
- `test_openai_format_request` — Message → OpenAI `messages` 格式，system 在 messages 数组中
- `test_openai_format_tool_definition` — ToolDef → OpenAI `functions` / `tools` 格式
- `test_openai_parse_stream_delta` — SSE `data:{"choices":[{"delta":{"content":"hi"}}]}` → StreamEvent::Delta
- `test_openai_parse_stream_tool_call` — `delta.tool_calls[0]` → StreamEvent::ToolCall
- `test_openai_parse_stream_done` — SSE `data:[DONE]` → StreamEvent::Done
- `test_openai_error_mapping` — 429/401/503 → 对应 ModelError

### 3.3 GeminiProvider

**Unit Tests** (`forge-model/src/gemini.rs`):
- `test_gemini_base_url` — GeminiProvider.base_url() 指向 `generativelanguage.googleapis.com/v1beta/openai/`
- `test_gemini_auth_header` — 请求头用 `x-goog-api-key` 而非 `Authorization: Bearer`
- `test_gemini_format_request` — 复用 OpenAI 格式，验证请求 body 一致
- `test_gemini_parse_stream` — 流式响应解析走 OpenAI 同一逻辑

**Integration Tests** (`forge-model/tests/gemini.rs`, `#[ignore]`):
- `test_gemini_real_stream` — 真实 Gemini API key → 收到响应

**Refactor**: 提取三个 Provider 共同的 SSE 解析逻辑到 `SseParser` 结构

---

## 迭代 4: Tool — 框架 + 只读工具 (2 天)

**测试目标**: `Tool` trait + ToolRegistry + 只读工具实现

### 4.1 工具注册框架

**Unit Tests** (`forge-tools/src/registry.rs`):
- `test_tool_registry_register` — registry.register(MockTool{name:"read"}) → registry.get("read") == Some
- `test_tool_registry_duplicate_error` — 注册两个 name="read" → Err(DuplicateTool("read"))
- `test_tool_registry_list_all` — 注册 3 个工具 → list() 返回 3 个 ToolDefinition{name, description, schema}
- `test_tool_schema_validation_pass` — execute(args={"path":"/a"}) 符合 schema → 正常执行
- `test_tool_schema_validation_fail` — execute(args={}) 缺少必填字段 → Err(ValidationError)
- `test_tool_timeout_enforcement` — MockTool.execute 内部 sleep(5s), timeout=1s → Err(TimeoutError)

### 4.2 Read / Glob / Grep（3 个 Tool trait 实现）

**Fixture**: 每个测试构建 `tempfile::TempDir`
```rust
fn setup() -> TempDir {
    let dir = tempdir().unwrap();
    write(dir/"main.rs", "fn main() {}\n");
    write(dir/"lib.rs", "pub fn hello() {}\n");
    mkdir(dir/"src/");
    write(dir/"src/utils.rs", "// utils\n");
    write(dir/".gitignore", "target/\n");
    mkdir(dir/"target/");
    write(dir/"target/debug.bin", <binary bytes>);
    dir
}
```

**Unit Tests — Read** (`forge-tools/src/read.rs`):
- `test_read_full_file` — read(path="main.rs") → content 包含 "fn main()"，行号从 1 开始
- `test_read_with_range` — read(path="main.rs", offset=1, limit=1) → 只返回第 1 行
- `test_read_file_not_found` — read(path="nonexist") → ToolOutput{is_error:true, content 含 "not found"}
- `test_read_binary_detection` — read(path="target/debug.bin") → ToolOutput{content 含 "binary file"}
- `test_read_empty_file` — read(空文件) → ToolOutput{content: ""}

**Unit Tests — Glob** (`forge-tools/src/glob.rs`):
- `test_glob_simple` — glob(pattern="*.rs", path=dir) → ["lib.rs", "main.rs"]
- `test_glob_recursive` — glob(pattern="**/*.rs") → ["main.rs", "lib.rs", "src/utils.rs"]
- `test_glob_no_match` — glob(pattern="*.py") → []
- `test_glob_respects_gitignore` — glob("**/*") 结果不含 "target/" 下的文件
- `test_glob_sorted_by_mtime` — touch main.rs 使其更新 → 结果中 main.rs 排在前面

**Unit Tests — Grep** (`forge-tools/src/grep.rs`):
- `test_grep_literal` — grep(pattern="fn main") → 匹配 main.rs:1
- `test_grep_regex` — grep(pattern="fn \\w+") → 匹配 main.rs 和 lib.rs
- `test_grep_with_context` — grep(pattern="main", context=1) → 返回匹配行 ± 1 行
- `test_grep_file_filter` — grep(pattern="fn", glob="lib.rs") → 只搜索 lib.rs
- `test_grep_case_insensitive` — grep(pattern="FN MAIN", case_insensitive=true) → 匹配
- `test_grep_no_match` — grep(pattern="zzz_not_exist") → 空结果

---

## 迭代 5: Tool — 写入工具 + 权限集成 (2 天)

**测试目标**: Write/Edit/Bash 的 `Tool` 实现 + ToolEngine 集成 `RuntimePrompter`

### 5.1 Write / Edit / Bash 工具

**Unit Tests — Write** (`forge-tools/src/write.rs`):
- `test_write_new_file` — write(path=dir/"new.rs", content="hi") → 文件存在且内容为 "hi"
- `test_write_overwrite` — 已有文件 → write(同路径, 新内容) → 内容被覆盖
- `test_write_creates_parent` — write(path=dir/"a/b/c.rs", ...) → 自动创建 a/b/ 目录
- `test_write_permission_denied` — write(path="/root/x") → ToolOutput{is_error:true}

**Unit Tests — Edit** (`forge-tools/src/edit.rs`):
- `test_edit_replace_exact` — 文件含 "old text" → edit(old="old text", new="new text") → 文件含 "new text"
- `test_edit_old_string_not_found` — edit(old="不存在的字符串") → ToolOutput{is_error:true, content 含 "not found"}
- `test_edit_old_string_ambiguous` — 文件含 2 处 "dup" → edit(old="dup") → ToolOutput{is_error:true, content 含 "ambiguous"}
- `test_edit_preserves_other_lines` — 编辑第 2 行 → 第 1、3 行不变

**Unit Tests — Bash** (`forge-tools/src/bash.rs`):
- `test_bash_stdout` — bash(command="echo hello") → ToolOutput{content:"hello\n", is_error:false}
- `test_bash_stderr` — bash(command="echo err >&2") → output 包含 "err"
- `test_bash_exit_code` — bash(command="exit 1") → ToolOutput{is_error:true, content 含退出码}
- `test_bash_timeout` — bash(command="sleep 10", timeout=100ms) → ToolOutput{is_error:true, content 含 "timeout"}
- `test_bash_working_dir` — bash(command="pwd", cwd=dir) → output 包含 dir 路径

### 5.2 ToolEngine + PermissionGateway 集成

```rust
pub struct ToolEngine<P: RuntimePrompter> {
    registry: ToolRegistry,
    gateway: PermissionGateway<P>,
}
```

**Integration Tests** (`forge-tools/tests/engine.rs`):
- `test_tool_engine_allow` — AutoAllowPrompter + ToolEngine → execute("read", {path}) → 正常返回文件内容
- `test_tool_engine_deny` — AutoDenyPrompter + ToolEngine → execute("write", {path}) → Err(PermissionDenied)
- `test_bash_readonly_blocked` — profile=ReadOnly → execute("bash", {command:"ls"}) → Deny，不执行命令
- `test_bash_git_auto_allowed` — rules 含 AutoAllow("git *") → execute("bash", {command:"git status"}) → 正常执行，不问用户
- `test_bash_rm_rf_denied` — rules 含 AlwaysDeny("rm -rf *") → execute("bash", {command:"rm -rf /"}) → Deny

---

## 迭代 6: MemoryRetriever + ContextEngine — Phase 1 实现 (2 天)

**测试目标**: `MemoryRetriever`(ForgemdRetriever) + `ContextEngine`(简单截断) + `CompactionProvider`(Noop)

### 6.1 ForgemdRetriever（MemoryRetriever 的 Phase 1 实现）

**Unit Tests** (`forge-memory/src/forgemd.rs`):
- `test_forgemd_load_global` — tempdir 创建 ~/.codeforge/FORGE.md 含 "global rules" → retrieve("") → MemoryChunk{content 含 "global rules"}
- `test_forgemd_load_project` — tempdir 创建 {project}/.codeforge/FORGE.md 含 "project rules" → retrieve("") → 含 "project rules"
- `test_forgemd_merge_global_and_project` — 两个 FORGE.md 都存在 → retrieve 返回合并内容，project 追加在 global 之后
- `test_forgemd_not_found_ok` — FORGE.md 不存在 → retrieve("") → 空 Vec（不 panic、不 Err）
- `test_forgemd_retrieve_ignores_query` — retrieve("any query") 和 retrieve("") 返回相同内容（Phase 1 不做语义检索）
- `test_forgemd_index_is_noop` — index(&[any_path]).await == Ok(())，无副作用

**Unit Tests** (`forge-memory/src/session.rs`):
- `test_session_save_jsonl` — 保存 3 条 Message → 文件有 3 行 JSON
- `test_session_load_jsonl` — 从 3 行 JSONL 文件加载 → 得到 3 条 Message，字段正确
- `test_session_roundtrip` — save → load → 逐字段比较 == 原始 messages
- `test_session_append` — 先 save 2 条 → 再 append 1 条 → load 得到 3 条

### 6.2 ContextEngine 实现（消费 MemoryRetriever + CompactionProvider）

**Unit Tests** (`forge-core/src/context.rs`):
- `test_assemble_system_prompt_always` — assemble(messages=[], budget=4096) → 结果[0].role == System
- `test_assemble_tool_definitions` — 注册 2 个工具 → assemble 的 system prompt 含两个工具的 JSON Schema

**Integration Tests** (`forge-core/tests/context.rs`):
- `test_assemble_within_budget` — 10 轮对话，budget=1000 tokens → assemble 结果的 token_counter ≤ 1000
- `test_assemble_forge_md_in_system` — MockMemoryRetriever.retrieve 返回 "FORGE content" → assemble 的 system prompt 含 "FORGE content"
- `test_assemble_empty_retriever` — MockMemoryRetriever.retrieve 返回 vec![] → assemble 正常，system prompt 不含 memory 内容
- `test_assemble_recent_3_turns_kept` — 10 轮对话，budget 不够 → 最近 3 轮的 content 完整出现在结果中
- `test_assemble_old_turns_truncated` — 10 轮对话，budget 不够 → 第 1-7 轮被截断丢弃（Phase 1 不摘要）
- `test_compact_with_noop_compaction` — MockCompactionProvider(Noop) → compact 结果只保留最近 N 轮，旧的被丢弃
- `test_compact_preserves_recent` — compact(messages=10轮, target=500) → 最近 3 轮完整保留

**Mock**: `MockMemoryRetriever` 控制 retrieve 返回值，`MockCompactionProvider` 控制 summarize 返回值

---

## 迭代 7: Agent Loop — 集成全部 Trait (2 天)

**测试目标**: Agent Loop 消费 `ModelProvider` + `ContextEngine` + `Tool`，全部用 mock

### 关键结构

```rust
pub struct AgentLoop<M: ModelProvider, C: ContextEngine, T: ToolExecutor> {
    model: M, context: C, tools: T, max_tool_rounds: usize,
}
```

**Integration Tests** (`forge-core/tests/agent.rs`，全部依赖用 mock):

基本流程：
- `test_agent_text_response` — MockModel 返回 [Delta("hi"), Done] → AgentLoop.run("input") → 输出 "hi"，循环结束
- `test_agent_single_tool_call` — MockModel 第 1 次返回 [ToolCall{name:"read",args:{path:"/a"}}]，MockTool 返回 "file content"，MockModel 第 2 次返回 [Delta("结果是..."), Done] → 输出 "结果是..."
- `test_agent_multiple_tool_calls` — MockModel 返回 2 个 ToolCall → MockTool 被调用 2 次 → 两个 ToolResult 都送回 MockModel

边界条件：
- `test_agent_tool_loop_depth_limit` — MockModel 每次都返回 ToolCall，max_tool_rounds=3 → 第 4 次不调模型，返回错误提示
- `test_agent_tool_error_reported` — MockTool.execute 返回 Err → messages 中包含 ToolResult{content:"error msg", is_error:true} → MockModel 看到错误信息
- `test_agent_permission_deny_reported` — MockTool 权限被拒 → ToolResult{content 含 "permission denied"} 送回 MockModel

流式和持久化：
- `test_agent_streaming_events` — 收集所有 AgentEvent → 按序为 [StreamStart, Delta, Delta, Done]
- `test_agent_saves_session` — AgentLoop.run 完成后 → session JSONL 文件存在且包含 user + assistant 消息

错误恢复：
- `test_agent_model_error_retry` — MockModel 第 1 次返回 Err(503)，第 2 次返回 Ok → 最终成功
- `test_agent_model_error_fatal` — MockModel 返回 Err(401) → 不重试，直接返回 AuthError
- `test_agent_context_overflow` — MockContextEngine.assemble 返回超过 model 上限的 messages → 触发 compact → 再次 assemble → 成功

**Mock 详情**:
- `MockModelProvider` — `.expect_chat_stream().times(N).returning(|req| ...)` 按调用次序返回不同响应
- `MockContextEngine` — `.expect_assemble().returning(|msgs, _| msgs.clone())` 透传或截断
- `MockToolExecutor` — `.expect_execute().returning(|call| ...)` 按 tool name 返回预设结果

---

## 迭代 8: RuntimePrompter — TUI 实现 (2 天)

**测试目标**: `RuntimePrompter` 的 `TuiRuntimePrompter` 实现

**状态与渲染分离**，测试状态逻辑不测渲染：
```rust
pub struct AppState { input: InputBuffer, messages: Vec<DisplayMessage>, scroll: usize }
```

**Unit Tests — InputBuffer** (`forge-tui/src/input.rs`):
- `test_input_insert_char` — buf="" → insert('a') → buf="a", cursor=1
- `test_input_backspace` — buf="ab", cursor=2 → backspace → buf="a", cursor=1
- `test_input_backspace_empty` — buf="" → backspace → buf="" (不 panic)
- `test_input_cursor_left` — buf="ab", cursor=2 → left → cursor=1
- `test_input_cursor_right_at_end` — buf="ab", cursor=2 → right → cursor=2 (不越界)
- `test_input_submit` — buf="hello" → submit() → 返回 "hello", buf="" 清空
- `test_input_multiline` — buf="line1" → shift_enter → buf="line1\n", cursor 在新行

**Unit Tests — DisplayMessage** (`forge-tui/src/display.rs`):
- `test_display_text_message` — Message{role:Assistant, content:Text("hi")} → DisplayMessage 可渲染
- `test_display_tool_call` — ToolCall{name:"read"} → 显示 "[read] path=/a"
- `test_display_tool_result` — ToolResult{content:"file..."} → 显示摘要（折叠长内容）
- `test_display_streaming_delta` — 逐个 Delta 追加 → DisplayMessage.content 逐步增长

**Integration Tests — TuiRuntimePrompter** (`forge-tui/tests/prompter.rs`):
- `test_tui_prompter_allow` — 模拟用户按 'y' → TuiRuntimePrompter.ask() → Permit
- `test_tui_prompter_deny` — 模拟用户按 'n' → Deny
- `test_tui_prompter_always_allow` — 模拟用户按 'a' → Permit + 验证生成了新 Rule

---

## 迭代 9: Tool — MCP 动态工具基础版 (1-2 天)

**测试目标**: 通过 MCP Client 将外部工具注册为 `Tool` trait 实现

> Phase 1 仅实现最小 MCP Client（connect + list + call），Phase 3 迭代 14 再完善生命周期管理和完整协议。

**Unit Tests** (`forge-mcp/src/client.rs`):
- `test_mcp_jsonrpc_serialize` — 构造 Request{method:"tools/list"} → JSON 包含 "jsonrpc":"2.0"
- `test_mcp_jsonrpc_parse_response` — 解析 `{"result":{"tools":[...]}}` → Vec<ToolDef>

**Integration Tests** (`forge-mcp/tests/client.rs`, 用 `tokio::io::duplex` mock stdio):
- `test_mcp_client_connect` — duplex 管道模拟 Server → Client.connect() → Ok
- `test_mcp_client_list_tools` — mock Server 响应 tools/list → Client 收到 Vec<ToolDef>
- `test_mcp_client_call_tool` — Client.call("search", {query:"x"}) → mock Server 响应结果 → 返回 ToolOutput
- `test_mcp_client_timeout` — mock Server 不响应 → 超时 → Err(Timeout)
- `test_mcp_client_server_crash` — duplex 写端 drop → Client 收到 Err(ConnectionClosed)
- `test_mcp_tool_as_trait` — McpTool{client, def} impl Tool → name()/schema() 正确 → execute() 内部调 Client.call()

---

## 迭代 10: 端到端集成 (2 天)

**测试目标**: 全部 trait 组装 + CLI + E2E 验证

### CLI 参数 + 模块组装

**Unit Tests** (`forge-cli/src/config.rs`):
- `test_cli_parse_model` — args: `--model claude-sonnet-4-20250514` → config.model == "claude-sonnet-4-20250514"
- `test_cli_parse_profile` — args: `--profile readonly` → config.profile == ReadOnly
- `test_cli_default_values` — 无参数 → model=默认值, profile=Coding
- `test_cli_config_from_toml` — config.toml 写入 `model = "gpt-4o"` → config.model == "gpt-4o"
- `test_cli_precedence` — config.toml 写 model=A, 环境变量 MODEL=B, CLI --model=C → config.model == C（CLI > env > file > default）

### 端到端集成测试（ScriptedModelProvider + 真实工具 + ForgemdRetriever）

**Integration Tests** (`forge-cli/tests/e2e.rs`):
- `test_e2e_simple_chat` — ScriptedModel 返回 "hello" → AgentLoop.run("hi") → 输出含 "hello"
- `test_e2e_read_file` — ScriptedModel 返回 ToolCall{name:"read", args:{path:test_file}} → 真实 ReadTool 执行 → ScriptedModel 看到文件内容 → 返回最终回复
- `test_e2e_write_file` — ScriptedModel 返回 ToolCall{name:"write"} + AutoAllowPrompter → tempdir 中文件被创建
- `test_e2e_tool_chain` — ScriptedModel 先返回 read ToolCall，再返回 edit ToolCall → 文件先读后改 → 验证文件内容
- `test_e2e_session_resume` — run 第 1 轮 → session JSONL 写入 → 加载 session → run 第 2 轮 → 历史中包含第 1 轮的 messages
- `test_e2e_forge_md_loaded` — tempdir 创建 FORGE.md 含 "custom rule" → ScriptedModel 接收到的 system prompt 含 "custom rule"

---

## 测试基础设施: forge-test-utils

```rust
pub fn create_test_project() -> TempDir;              // 临时项目目录
pub struct ScriptedModelProvider { responses: VecDeque<ChatResponse> }  // 按剧本返回
pub struct AutoAllowPrompter;                          // 自动放行（RuntimePrompter 实现）
pub struct AutoDenyPrompter;                           // 自动拒绝（RuntimePrompter 实现）
pub fn assert_tool_output_contains(output: &ToolOutput, expected: &str);
pub fn assert_has_message(messages: &[Message], role: Role, contains: &str);
```

---

# Phase 2: Memory（Noop → 真实实现）

## 迭代 11: EmbeddingProvider — 真实实现 + 向量存储 (2 天)

**测试目标**: 用真实 `EmbeddingProvider` 替换 `NoopEmbedding`

### 11.1 EmbeddingProvider 实现

**Unit Tests** (`forge-memory/src/embedding/openai.rs`):
- `test_openai_embedding_format_request` — embed(&["hello"]) → HTTP body 包含 `{"input":["hello"],"model":"text-embedding-3-small"}`
- `test_openai_embedding_parse_response` — 解析 API 响应 `{"data":[{"embedding":[0.1,0.2,...]}]}` → Vec<f32> 长度 == dimension()
- `test_openai_embedding_dimension` — OpenAIEmbedding::new("text-embedding-3-small").dimension() == 1536

**Unit Tests** (`forge-memory/src/embedding/gemini.rs`):
- `test_gemini_embedding_format` — 请求格式兼容验证
- `test_gemini_embedding_dimension` — dimension() 返回正确维度

**Integration Tests** (`forge-memory/tests/embedding.rs`):
- `test_embedding_batch` — MockEmbedding(dim=3).embed(&["a","b"]) → 2 个向量，每个长度 3
- `test_embedding_dimension_mismatch` — MockEmbedding(dim=3) 返回 dim=5 的向量 → Err(DimensionMismatch)
- `test_openai_real_embedding` — (`#[ignore]`) 真实 API → 返回非空向量

### 11.2 SQLite-vec 向量存储 + FTS5 全文检索

**Unit Tests — 向量存储** (`forge-memory/src/vec_store.rs`, 每个测试用临时 SQLite 文件):
- `test_sqlite_vec_insert` — insert(id="doc1", vec=[0.1,0.2,0.3]) → query 返回 doc1
- `test_sqlite_vec_knn_query` — 插入 3 个向量 → knn(query_vec, k=2) → 返回最近的 2 个，按相似度降序
- `test_sqlite_vec_empty_table` — knn(任意向量, k=10) → 空 Vec
- `test_sqlite_vec_delete` — insert doc1 → delete doc1 → knn 不再返回 doc1

**Unit Tests — FTS5** (`forge-memory/src/fts_store.rs`, 每个测试用临时 SQLite 文件):
- `test_fts5_index_document` — index(id="doc1", text="rust programming") → Ok
- `test_fts5_search_keyword` — index 3 篇文档 → search("rust") → 返回含 "rust" 的文档
- `test_fts5_search_phrase` — search('"rust programming"') → 精确匹配短语
- `test_fts5_search_boolean` — search("rust AND NOT python") → 只返回含 rust 不含 python 的文档
- `test_fts5_rank_bm25` — 2 篇文档，一篇 "rust" 出现 5 次，另一篇 1 次 → 排序正确
- `test_fts5_cjk_tokenizer` — index(text="Rust 编程语言") → search("编程") → 匹配到

---

## 迭代 12: MemoryRetriever — HybridRetriever 替换 ForgemdRetriever (2 天)

**测试目标**: 用 `HybridRetriever` 替换 `ForgemdRetriever`，验证 `MemoryRetriever` trait 可插拔

### 12.1 HybridRetriever（MemoryRetriever 的 Phase 2 实现）

**关键结构**:
```rust
pub struct HybridRetriever {
    vec_store: SqliteVecStore,
    fts_store: Fts5Store,
    embedding: Box<dyn EmbeddingProvider>,  // 消费 EmbeddingProvider trait
    vec_weight: f32,
}
impl MemoryRetriever for HybridRetriever { ... }
```

**Unit Tests** (`forge-memory/src/hybrid.rs`):
- `test_hybrid_rrf_ranking` — vec 排序 [A,B,C], fts 排序 [B,A,D] → RRF 合并去重 → 排序正确（B 两端都靠前所以排第一）
- `test_hybrid_vec_weight` — vec_weight=1.0 → 结果接近纯向量排序；vec_weight=0.0 → 接近纯 FTS 排序
- `test_hybrid_top_k` — 索引 10 篇文档 → retrieve(query, top_k=3) → 返回恰好 3 个结果

**Integration Tests** (`forge-memory/tests/hybrid.rs`, MockEmbeddingProvider + 真实 SQLite):
- `test_hybrid_index_and_retrieve` — index 3 个文件 → retrieve("keyword") → 返回含相关内容的 MemoryChunk
- `test_hybrid_empty_query` — retrieve("") → 空 Vec
- `test_hybrid_filters_by_scope` — index global + project 文件 → retrieve(scope=Project) → 只返回 project 的

### 12.2 可插拔验证

**Integration Tests** (`forge-core/tests/retriever_swap.rs`):
- `test_swap_forgemd_to_hybrid` — ContextEngine 先用 ForgemdRetriever → 换成 HybridRetriever → assemble 仍然正常，system prompt 含 memory 内容
- `test_swap_to_vec_only` — VecOnlyRetriever impl MemoryRetriever → ContextEngine 正常
- `test_swap_to_fts_only` — FtsOnlyRetriever impl MemoryRetriever → ContextEngine 正常
- `test_custom_retriever` — 自定义 struct MyRetriever impl MemoryRetriever{返回固定内容} → ContextEngine.assemble 含该固定内容

---

## 迭代 13: CompactionProvider + 增量索引 + memory 工具 (2 天)

**测试目标**: 用 `LlmCompaction` 替换 `NoopCompaction` + memory 工具作为 `Tool` 实现

### 13.1 LlmCompaction（CompactionProvider 的 Phase 2 实现）

**Unit Tests** (`forge-memory/src/compaction.rs`):
- `test_llm_compaction_prompt` — LlmCompaction.summarize(messages) → 验证发给 MockModel 的 prompt 包含 "summarize the following conversation"
- `test_llm_compaction_returns_summary` — MockModel 返回 "用户讨论了文件读写" → summarize() == Ok("用户讨论了文件读写")

**Integration Tests** (`forge-core/tests/compaction.rs`):
- `test_llm_compaction_reduces_tokens` — 10 轮对话(~2000 tokens) → summarize → 返回字符串 token 数 < 500
- `test_llm_compaction_preserves_key_info` — messages 含 ToolCall{name:"write",path:"/a.rs"} → 摘要中包含 "/a.rs"（MockModel 返回含路径的摘要）
- `test_context_engine_uses_llm_compaction` — ContextEngine 注入 LlmCompaction → compact() → MockModel.chat_stream 被调用（而非直接截断）

**Mock**: `MockModelProvider` — `.expect_chat_stream().returning(|_| Ok(stream_of("摘要内容")))`

### 13.2 实时增量索引

**Unit Tests** (`forge-memory/src/indexer.rs`):
- `test_incremental_index_new_file` — indexer.on_file_change(Created, "new.rs") → vec_store 和 fts_store 中能检索到 new.rs 内容
- `test_incremental_index_modified_file` — 修改文件内容 → on_file_change(Modified, "a.rs") → 旧向量被删除，新向量被插入
- `test_incremental_index_deleted_file` — on_file_change(Deleted, "a.rs") → 检索不到 a.rs

**Integration Tests** (`forge-memory/tests/indexer.rs`):
- `test_incremental_index_async` — spawn indexer 后台任务 → 发送 3 个文件变更 → 主线程不阻塞 → await 后索引完成
- `test_index_conversation_turn` — indexer.on_turn_end(messages) → 新消息被索引 → retrieve 能找到

### 13.3 memory_search / memory_save（2 个新 Tool 实现）

**Unit Tests** (`forge-tools/src/memory_search.rs`):
- `test_memory_search_schema` — MemorySearchTool.schema() 包含 "query"(required) 和 "scope"(optional) 字段
- `test_memory_search_name` — MemorySearchTool.name() == "memory_search"

**Integration Tests** (`forge-tools/tests/memory_tools.rs`, MockMemoryRetriever):
- `test_memory_search_returns_chunks` — MockRetriever 返回 2 个 MemoryChunk → execute({query:"x"}) → ToolOutput.content 含 2 段内容
- `test_memory_search_with_scope` — execute({query:"x", scope:"project"}) → MockRetriever.retrieve 的 opts.scope == Project
- `test_memory_save_writes` — execute({content:"new rule"}) → tempdir/FORGE.md 文件末尾含 "new rule"
- `test_memory_save_append` — FORGE.md 已有 "old" → execute({content:"new"}) → 内容为 "old\nnew"
- `test_context_engine_retriever_integration` — HybridRetriever(已索引) + ContextEngine → assemble 的 system prompt 含检索到的 memory 内容

---

# Phase 3: 扩展

## 迭代 14: Tool — MCP Client 完善 (2 天)

**测试目标**: MCP 动态工具的完整 `Tool` trait 生命周期

### 14.1 MCP 生命周期管理

**Unit Tests** (`forge-mcp/src/manager.rs`):
- `test_server_config_parse` — toml `[[mcp_servers]]\nname="web"\ncommand="node"\nargs=["server.js"]` → ServerConfig 正确解析
- `test_server_config_invalid` — 缺少 command 字段 → Err(ConfigError)
- `test_restart_policy_max_3` — RestartPolicy{max:3, count:3}.should_restart() == false
- `test_restart_policy_within_limit` — RestartPolicy{max:3, count:1}.should_restart() == true

**Integration Tests** (`forge-mcp/tests/lifecycle.rs`, 用真实子进程):
- `test_mcp_server_start` — ServerManager.start("echo-server") → 子进程 PID > 0，状态 == Running
- `test_mcp_server_stop` — start → stop() → 子进程退出，状态 == Stopped
- `test_mcp_server_stop_force` — start → stop() 后 2s 仍未退出 → SIGKILL → 状态 == Stopped
- `test_mcp_server_auto_restart` — start → kill 子进程 → 等 1s → 新 PID 出现，restart_count == 1
- `test_mcp_server_restart_exhausted` — 连续 crash 3 次 → 状态 == Failed，不再重启
- `test_mcp_multiple_servers` — start("server-a") + start("server-b") → 两个都 Running，互不影响
- `test_mcp_server_isolation` — kill server-a → server-b 仍 Running

### 14.2 MCP 协议完整实现

**Unit Tests** (`forge-mcp/src/protocol.rs`):
- `test_mcp_initialize_request` — 构造 initialize 请求 → JSON 包含 protocolVersion + capabilities
- `test_mcp_parse_initialize_response` — 解析 `{"capabilities":{"tools":true}}` → ServerCapabilities 正确

**Integration Tests** (`forge-mcp/tests/protocol.rs`, duplex mock):
- `test_mcp_initialize_handshake` — Client → initialize → mock Server 响应 capabilities → Client.capabilities.tools == true
- `test_mcp_resources_list` — Client.resources_list() → mock 返回 2 个 Resource → Vec 长度 2
- `test_mcp_resources_read` — Client.resources_read("file://a.rs") → mock 返回文件内容 → content == "fn main()"
- `test_mcp_prompts_list` — Client.prompts_list() → mock 返回 prompt 模板列表
- `test_mcp_prompts_get` — Client.prompts_get("review") → mock 返回 prompt 内容
- `test_mcp_notification` — mock Server 发送 notification → Client.on_notification 回调被触发

---

## 迭代 15: Tool — Skills 目录支持 (2 天)

**测试目标**: Skills 作为 `Tool` trait 的另一种注册来源

### 15.1 Skills 扫描 + 注册

**Meta 格式**:
```bash
#!/bin/bash
# @skill: commit
# @description: Create a git commit with conventional format
# @usage: /commit [message]
```

**Unit Tests** (`forge-tools/src/skills/scanner.rs`):
- `test_skills_parse_meta_shell` — 文件头含 `# @skill: commit\n# @description: ...` → SkillMeta{name:"commit", desc:"...", usage:"/commit [message]"}
- `test_skills_parse_meta_python` — `# @skill: lint` → SkillMeta{name:"lint"}
- `test_skills_parse_no_meta` — 文件无 `@skill` 标记 → None（跳过）
- `test_skills_parse_partial_meta` — 只有 `@skill` 无 `@description` → SkillMeta{desc: ""}（不报错）

**Integration Tests** (`forge-tools/tests/skills.rs`, tempdir 含脚本文件):
- `test_skills_scan_directory` — tempdir/skills/ 下放 3 个脚本（2 有 meta，1 无） → scan() → 返回 2 个 SkillMeta
- `test_skills_scan_empty_dir` — 空目录 → scan() → 空 Vec
- `test_skills_inject_system_prompt` — 2 个 SkillMeta → inject_to_prompt(system) → system prompt 含 "/commit" 和 "/lint" 的描述
- `test_skills_register_as_tool` — scan 后的每个 Skill impl Tool → ToolRegistry.get("commit") == Some

### 15.2 Skills 执行

**Unit Tests** (`forge-tools/src/skills/executor.rs`):
- `test_skill_build_command` — SkillExecutor.build_cmd(skill, args) → Command{program:"/path/to/commit.sh", args:["fix typo"], cwd:project_root}
- `test_skill_env_vars` — build_cmd → env 包含 PROJECT_DIR=project_root, SKILL_NAME="commit"

**Integration Tests** (`forge-tools/tests/skills_exec.rs`, tempdir 含真实脚本):
- `test_skill_execute_success` — tempdir 创建 `echo "done"` 的脚本 → execute({}) → ToolOutput{content:"done\n", is_error:false}
- `test_skill_execute_working_dir` — 脚本内容 `pwd` → execute(cwd=tempdir) → output 含 tempdir 路径
- `test_skill_timeout` — 脚本内容 `sleep 10` → execute(timeout=100ms) → ToolOutput{is_error:true, content 含 "timeout"}
- `test_skill_permission_check` — AutoDenyPrompter + ToolEngine → execute skill → Err(PermissionDenied)

---

## 迭代 16: 内置 MCP Server 插件 (2-3 天)

**测试目标**: TS 实现的 MCP Server + 与 forge-mcp client 的集成

### 16.1 web_search MCP Server (TS/Jest)

**Unit Tests** (`packages/mcp-servers/web-search/__tests__/`):
- `test_web_search_tool_list` — Server.tools_list() → 包含 {name:"web_search", inputSchema:{query: string}}
- `test_web_search_format_request` — tools_call({query:"rust lang"}) → 验证发往搜索 API 的请求格式
- `test_web_search_parse_response` — mock 搜索 API 返回 3 条结果 → 解析为 [{title, url, snippet}]
- `test_web_search_rate_limit` — 短时间发 100 次请求 → 第 N 次返回 error{code: "rate_limited"}
- `test_web_search_api_error` — mock API 返回 500 → tools_call 返回 {isError:true, content:"search API error"}

### 16.2 browser MCP Server (TS/Jest)

**Unit Tests** (`packages/mcp-servers/browser/__tests__/`):
- `test_browser_tool_list` — tools_list() → 包含 navigate/snapshot/click/fill 等工具定义
- `test_browser_navigate` — tools_call("navigate", {url:"https://example.com"}) → Playwright page.goto 被调用
- `test_browser_snapshot` — tools_call("snapshot") → 返回页面 accessibility tree 文本
- `test_browser_click` — tools_call("click", {selector:"button#submit"}) → page.click 被调用
- `test_browser_fill` — tools_call("fill", {selector:"input#name", value:"test"}) → page.fill 被调用

### 集成测试 (Rust 端)

**Integration Tests** (`forge-mcp/tests/mcp_servers.rs`):
- `test_mcp_e2e_web_search` — 启动真实 web_search server 子进程 → Client.call("web_search", {query:"test"}) → 返回搜索结果（`#[ignore]`，需网络）
- `test_mcp_e2e_browser` — 启动真实 browser server → Client.call("navigate", {url:"data:text/html,<h1>hi</h1>"}) → snapshot 含 "hi"

---

# Phase 4: IDE + 部署

## 迭代 17: RuntimePrompter — LSP 实现 (3 天)

**测试目标**: `RuntimePrompter` 的 `LspRuntimePrompter` 实现

### 17.1 LSP 基础协议

**Unit Tests** (`forge-lsp/src/handler.rs`):
- `test_lsp_capabilities` — ServerCapabilities 包含 textDocumentSync + 自定义 codeforge/* 方法
- `test_lsp_parse_initialize_params` — InitializeParams → 提取 workspace root、client capabilities

**Integration Tests** (`forge-lsp/tests/protocol.rs`, 用 `tower_lsp::LspService` + mock transport):
- `test_lsp_initialize` — Client 发送 initialize → Server 返回 capabilities → 包含 codeforge/chat 支持
- `test_lsp_shutdown` — Client 发送 shutdown → Server 返回 Ok → Client 发送 exit → 连接关闭
- `test_lsp_text_document_open` — Client 发送 didOpen{uri, text} → Server 内部记录文档
- `test_lsp_text_document_change` — Client 发送 didChange → Server 内部更新文档内容
- `test_lsp_notification_progress` — Server 处理长任务 → Client 收到 $/progress 通知

### 17.2 Agent 集成 + LspRuntimePrompter

**关键结构**:
```rust
pub struct LspRuntimePrompter { client: tower_lsp::Client }
impl RuntimePrompter for LspRuntimePrompter { ... }
```

**Unit Tests** (`forge-lsp/src/prompter.rs`):
- `test_lsp_prompter_format_request` — ask("bash","git status") → showMessageRequest 内容含 "Allow bash: git status?" + 3 个选项 [Allow/Deny/Always]

**Integration Tests** (`forge-lsp/tests/agent.rs`, MockModelProvider + mock LSP Client):
- `test_lsp_chat_request` — Client 发送 codeforge/chat{message:"hi"} → ScriptedModel 返回 "hello" → Client 收到 response 含 "hello"
- `test_lsp_tool_approval` — ScriptedModel 返回 ToolCall → Server 发送 showMessageRequest → mock Client 回复 Allow → 工具执行 → 结果回传
- `test_lsp_prompter_deny` — mock Client 回复 Deny → 工具不执行 → ToolResult 含 "permission denied"
- `test_lsp_streaming` — ScriptedModel 返回多个 Delta → Client 逐个收到 $/progress 或 partial result
- `test_lsp_multi_workspace` — 2 个 workspace root → 各自独立 AgentLoop 实例 → 互不干扰

### 17.3 IDE 兼容性

**Integration Tests** (`forge-lsp/tests/ide_compat.rs`):
- `test_lsp_vscode_lifecycle` — 模拟 VS Code 启动序列：initialize → initialized → didOpen → codeforge/chat → shutdown → exit
- `test_lsp_neovim_lifecycle` — 模拟 Neovim 启动序列（capabilities 略有不同）→ 同样走通

---

## 迭代 18: RuntimePrompter — gRPC 实现 + Gateway (3 天)

**测试目标**: `RuntimePrompter` 的 `GrpcRuntimePrompter` 实现 + Daemon

### 18.1 Daemon 进程管理

**Unit Tests** (`forge-gateway/src/daemon.rs`):
- `test_daemon_pid_file_path` — Daemon::pid_path() == ~/.codeforge/daemon.pid
- `test_daemon_parse_pid_file` — 文件内容 "12345\n" → pid == 12345
- `test_daemon_is_running_check` — pid 文件存在 + 进程存活 → true；pid 文件存在 + 进程不存在 → false（清理 stale pid）

**Integration Tests** (`forge-gateway/tests/daemon.rs`):
- `test_daemon_start` — Daemon::start() → pid 文件被创建 → 进程存活 → health_check() == Ok
- `test_daemon_stop` — start → stop() → pid 文件被删除 → 进程退出
- `test_daemon_already_running` — start → 再 start → Err(AlreadyRunning{pid})
- `test_daemon_health_check` — start → HTTP GET /health → 200 OK {status:"healthy"}
- `test_daemon_auto_shutdown` — start(idle_timeout=1s) → 不连接任何 client → 1s 后进程自动退出

### 18.2 gRPC Gateway + GrpcRuntimePrompter

**Unit Tests** (`forge-gateway/src/prompter.rs`):
- `test_grpc_prompter_format` — ask("write","path=/a") → 构造 ToolApprovalRequest{tool:"write", args:"path=/a"}
- `test_grpc_prompter_parse_response` — ToolApprovalResponse{decision:"allow"} → PermissionDecision::Permit

**Integration Tests** (`forge-gateway/tests/grpc.rs`, 启动真实 gRPC server on localhost):
- `test_grpc_chat_stream` — Client.chat_stream({message:"hi"}) → ScriptedModel 返回 "hello" → Client 收到流式 response
- `test_grpc_session_create` — Client.create_session() → 返回 session_id
- `test_grpc_session_resume` — create → chat → 新连接 resume(session_id) → 历史消息保留
- `test_grpc_session_list` — 创建 3 个 session → list_sessions() → 返回 3 个
- `test_grpc_prompter_approval` — ScriptedModel 返回 ToolCall → Server 发送 ToolApprovalRequest → Client 回复 Allow → 工具执行
- `test_grpc_prompter_deny` — Client 回复 Deny → ToolResult 含 "permission denied"
- `test_grpc_multi_client` — 2 个 Client 同时连接不同 session → 互不干扰
- `test_grpc_auth` — 无 token 连接 → Err(Unauthenticated)；正确 token → 连接成功
- `test_grpc_cancel` — Client 开始 chat_stream → 中途 drop stream → Server 优雅停止，无 panic

### 18.3 Channel 集成测试

**Integration Tests** (`forge-gateway/tests/channel.rs`, 启动 Daemon + gRPC server):
- `test_channel_cli_to_gateway` — CLI 进程通过 gRPC 连接 → chat("hi") → 收到回复（ScriptedModel）
- `test_channel_lsp_to_gateway` — LSP Server 内部通过 gRPC 连接 Gateway → codeforge/chat → 收到回复
- `test_channel_session_sharing` — CLI 创建 session + 发消息 → LSP resume 同一 session → 看到 CLI 的历史消息

---

# Sprint 总览（全部 Phase）

| Sprint | 迭代 | 天数 | Phase | 交付物 | 涉及 Trait |
|--------|------|------|-------|--------|-----------|
| S1 | 0 + 1 | 3 天 | P1 | 骨架 + 7 trait 定义 + Noop 实现 + forge-permissions | 全部 trait 定义 + RuntimePrompter |
| S2 | 2 + 3 | 4 天 | P1 | forge-model (Router + 3 Providers) | ModelProvider |
| S3 | 4 + 5 | 4 天 | P1 | forge-tools + 权限集成 | Tool + RuntimePrompter |
| S4 | 6 + 7 | 4 天 | P1 | forge-memory(简化) + Agent Loop | MemoryRetriever + ContextEngine + CompactionProvider(Noop) |
| S5 | 8 | 2 天 | P1 | forge-tui | RuntimePrompter(TUI 实现) |
| S6 | 9 + 10 | 3-4 天 | P1 | forge-mcp(基础) + forge-cli + E2E | Tool(MCP) |
| S7 | 11 + 12 | 4 天 | P2 | 向量索引 + 混合检索 | EmbeddingProvider + MemoryRetriever(Hybrid) |
| S8 | 13 | 2 天 | P2 | LLM 压缩 + 增量索引 + memory 工具 | CompactionProvider(LLM) + Tool(memory) |
| S9 | 14 + 15 | 4 天 | P3 | MCP 完善 + Skills | Tool(MCP 完善 + Skills) |
| S10 | 16 | 2-3 天 | P3 | 内置 MCP Server 插件 | Tool(MCP 集成) |
| S11 | 17 | 3 天 | P4 | forge-lsp | RuntimePrompter(LSP 实现) |
| S12 | 18 | 3 天 | P4 | forge-gateway + Daemon | RuntimePrompter(gRPC 实现) |

**Phase 1: ~20 工作日（4 周）→ MVP**
**Phase 2: ~6 工作日（1.5 周）→ Memory**
**Phase 3: ~8 工作日（2 周）→ 扩展**
**Phase 4: ~6 工作日（1.5 周）→ IDE + 部署**
**总计: ~40 工作日（8 周）→ 全功能版本**

---

## 关键依赖

```toml
[workspace.dependencies]
# Phase 1: 基础
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
mockall = "0.13"
tempfile = "3"
anyhow = "1"
thiserror = "2"
clap = { version = "4", features = ["derive"] }
ratatui = "0.29"
rusqlite = { version = "0.32", features = ["bundled"] }
reqwest = { version = "0.12", features = ["stream", "json"] }
glob = "0.3"
regex = "1"
jsonschema = "0.26"
tokio-stream = "0.1"
futures = "0.3"
tracing = "0.1"

# Phase 2: Memory
sqlite-vec = "0.1"                     # 向量索引

# Phase 4: IDE + 部署
tower-lsp = "0.20"                     # LSP Server
tonic = "0.12"                         # gRPC Gateway
prost = "0.13"                         # protobuf 编解码
```

---

## TDD 纪律检查清单

每次提交前：
1. 是否先写了测试再写实现？（Red 验证过？）
2. 实现是否是让测试通过的最少代码？
3. 重构时所有测试仍通过？
4. 新 trait 有对应 mock 测试？
5. 错误路径有测试覆盖？
6. 用了 `tempfile` 而非污染真实文件系统？
7. Noop 替换为真实实现时，旧测试是否仍通过？

---

## 验证方式

每个迭代完成后：
- `cargo test -p forge-xxx` — 对应 crate 全部测试通过
- `cargo test --workspace` — 无回归
- `cargo clippy --workspace` — 无 warning

Phase 里程碑验证：
- **Phase 1**: 手动运行 CLI，与 Claude/Gemini API 完成一次完整工具调用对话（Noop Memory/Compaction）
- **Phase 2**: 多轮对话后验证 memory_search 能检索到早期上下文，compact 用 LLM 摘要
- **Phase 3**: 配置外部 MCP Server，验证工具发现 + 调用 + 结果回传；运行 skill 脚本
- **Phase 4**: VS Code / Neovim 通过 LSP 连接 CodeForge，完成端到端对话；CLI 和 IDE 共享 session
