pub mod basic;
pub mod secure;
pub mod types;

use crate::{
    error::McpError,
    prompts::{ListPromptsResponse, PromptResult},
    protocol::ProtocolHandle,
    resource::{ListResourcesResponse, ReadResourceResponse},
    tools::{ListToolsResponse, ToolResult},
};
use async_trait::async_trait;
use serde_json::Value;

use types::{ClientInfo, InitializeResult, ServerCapabilities};

#[async_trait]
pub trait Client: Send + Sync {
    async fn connect<T: crate::transport::Transport + Send + Sync + 'static>(
        &mut self,
        transport: T,
    ) -> Result<ProtocolHandle, McpError>;

    async fn initialize(&mut self, client_info: ClientInfo) -> Result<InitializeResult, McpError>;

    async fn list_resources(
        &self,
        cursor: Option<String>,
    ) -> Result<ListResourcesResponse, McpError>;

    async fn read_resource(&self, uri: String) -> Result<ReadResourceResponse, McpError>;

    async fn subscribe_to_resource(&self, uri: String) -> Result<(), McpError>;

    async fn list_prompts(&self, cursor: Option<String>) -> Result<ListPromptsResponse, McpError>;

    async fn get_prompt(
        &self,
        name: String,
        arguments: Option<Value>,
    ) -> Result<PromptResult, McpError>;

    async fn list_tools(&self, cursor: Option<String>) -> Result<ListToolsResponse, McpError>;

    async fn call_tool(&self, name: String, arguments: Value) -> Result<ToolResult, McpError>;

    async fn set_log_level(&self, level: String) -> Result<(), McpError>;

    async fn shutdown(&mut self) -> Result<(), McpError>;

    async fn get_server_capabilities(&self) -> Option<ServerCapabilities>;

    async fn has_capability(&self, capability: &str) -> bool;

    async fn get_client_info(&self) -> Option<ClientInfo>;

    async fn has_client_info(&self) -> bool;

    async fn assert_initialized(&self) -> Result<(), McpError>;

    async fn assert_capability(&self, capability: &str) -> Result<(), McpError>;

    async fn cleanup_resources(&mut self) -> Result<(), McpError>;

    async fn wait_for_shutdown(&mut self) -> Result<(), McpError>;
}
