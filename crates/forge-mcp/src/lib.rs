pub mod protocol;
pub mod client;
pub mod tool;
pub mod manager;

pub use client::{McpClient, McpError};
pub use protocol::ToolDef;
pub use tool::McpTool;
pub use manager::{McpConfig, RestartPolicy, ServerConfig, ServerManager, ServerStatus};
