use crate::traits::*;
use crate::types::*;

/// 工具输出最大字符数（超过则截断，避免撑爆 context）。
const MAX_TOOL_OUTPUT_CHARS: usize = 30_000;

/// Agent Loop 产生的事件，用于驱动 TUI/LSP 渲染。
#[derive(Debug, Clone)]
pub enum AgentEvent {
    StreamStart,
    Delta { content: String },
    ToolCallStart { id: String, name: String },
    ToolResult { id: String, output: ToolOutput },
    Done,
}

/// Agent Loop：消费 ModelProvider + ContextEngine + ToolExecutor，
/// 实现"用户输入 → LLM 推理 → 工具执行 → 结果回传"的主循环。
pub struct AgentLoop<M, C, T>
where
    M: ModelProvider,
    C: ContextEngine,
    T: ToolExecutor,
{
    model: M,
    context: C,
    tools: T,
    max_tool_rounds: usize,
    model_name: String,
    session: Option<Box<dyn SessionStore>>,
    messages: Vec<Message>,
}

impl<M, C, T> AgentLoop<M, C, T>
where
    M: ModelProvider,
    C: ContextEngine,
    T: ToolExecutor,
{
    pub fn new(model: M, context: C, tools: T, max_tool_rounds: usize) -> Self {
        Self {
            model,
            context,
            tools,
            max_tool_rounds,
            model_name: "default".to_string(),
            session: None,
            messages: Vec::new(),
        }
    }

    pub fn with_model_name(mut self, name: impl Into<String>) -> Self {
        self.model_name = name.into();
        self
    }

    pub fn with_session(mut self, session: Box<dyn SessionStore>) -> Self {
        self.session = Some(session);
        self
    }

    /// 设置初始消息（用于 session resume）。
    pub fn set_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
    }

    /// 清空对话历史。
    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }

    /// 运行一轮对话。返回最终的 assistant 文本回复。
    pub async fn run(
        &mut self,
        input: &str,
        event_tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    ) -> anyhow::Result<String> {
        // 添加用户消息
        self.messages.push(Message {
            role: Role::User,
            content: Content::Text(input.to_string()),
            tool_calls: vec![],
        });

        let mut final_text = String::new();

        for round in 0..self.max_tool_rounds {
            // 编排上下文
            let budget = TokenBudget {
                max_tokens: self.model.capabilities().max_context_tokens,
                reserved: 0,
            };
            let mut assembled = self.context.assemble(&self.messages, budget).await;

            // 检测溢出 → compact → 重新 assemble
            let token_count = self.model.token_counter(&assembled);
            if token_count > self.model.capabilities().max_context_tokens {
                let target = TokenBudget {
                    max_tokens: self.model.capabilities().max_context_tokens,
                    reserved: 0,
                };
                self.messages = self.context.compact(&self.messages, target).await;
                assembled = self.context.assemble(&self.messages, budget).await;
            }

            // 构建请求
            let request = ChatRequest::builder()
                .model(&self.model_name)
                .messages(assembled)
                .tools(self.tools.tool_schemas())
                .build();

            // 调用模型（带重试）
            let mut stream = self.call_model_with_retry(request).await?;

            let _ = event_tx.send(AgentEvent::StreamStart);

            // 收集流式响应
            let mut text = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();

            while let Some(event) = stream.next().await {
                match event {
                    StreamEvent::Delta { content } => {
                        text.push_str(&content);
                        let _ = event_tx.send(AgentEvent::Delta { content });
                    }
                    StreamEvent::ToolCall {
                        id,
                        name,
                        arguments,
                    } => {
                        let _ = event_tx.send(AgentEvent::ToolCallStart {
                            id: id.clone(),
                            name: name.clone(),
                        });
                        tool_calls.push(ToolCall {
                            id,
                            name,
                            arguments,
                        });
                    }
                    StreamEvent::Done { .. } => {
                        let _ = event_tx.send(AgentEvent::Done);
                    }
                }
            }

            // 添加 assistant 消息
            self.messages.push(Message {
                role: Role::Assistant,
                content: Content::Text(text.clone()),
                tool_calls: tool_calls.clone(),
            });

            // 无工具调用 → 对话结束
            if tool_calls.is_empty() {
                final_text = text;
                break;
            }

            // 执行工具并回传结果
            for call in &tool_calls {
                let output = match self.tools.execute(call).await {
                    Ok(output) => output,
                    Err(e) => ToolOutput {
                        content: e.to_string(),
                        is_error: true,
                    },
                };

                let _ = event_tx.send(AgentEvent::ToolResult {
                    id: call.id.clone(),
                    output: output.clone(),
                });

                // Truncate large tool outputs to avoid context overflow
                let content = if output.content.len() > MAX_TOOL_OUTPUT_CHARS {
                    format!(
                        "{}...\n[truncated: {} chars total]",
                        &output.content[..MAX_TOOL_OUTPUT_CHARS],
                        output.content.len()
                    )
                } else {
                    output.content
                };

                self.messages.push(Message {
                    role: Role::User,
                    content: Content::ToolResult {
                        tool_use_id: call.id.clone(),
                        output: content,
                    },
                    tool_calls: vec![],
                });
            }

            // 最后一轮仍有工具调用 → 超出限制
            if round == self.max_tool_rounds - 1 {
                final_text =
                    "Error: exceeded maximum tool call rounds".to_string();
            }
        }

        // 持久化会话
        if let Some(store) = &self.session {
            store.save(&self.messages).await?;
        }

        Ok(final_text)
    }

    /// 调用模型，暂时性错误自动重试一次。
    async fn call_model_with_retry(
        &self,
        request: ChatRequest,
    ) -> anyhow::Result<StreamResponse> {
        match self.model.chat_stream(request.clone()).await {
            Ok(stream) => Ok(stream),
            Err(e) => {
                if let Some(ModelError::Transient { .. }) = e.downcast_ref::<ModelError>() {
                    // 暂时性错误，重试一次
                    self.model.chat_stream(request).await
                } else {
                    Err(e)
                }
            }
        }
    }
}
