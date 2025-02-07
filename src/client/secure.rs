use async_trait::async_trait;
use serde_json::Value;
use std::{collections::HashMap, env, sync::Arc};
use tokio::sync::RwLock;

use crate::{
    error::McpError,
    prompts::{ListPromptsResponse, PromptResult},
    protocol::ProtocolHandle,
    resource::{ListResourcesResponse, ReadResourceResponse},
    tools::{ListToolsResponse, ToolResult},
    transport::Transport,
};

use super::{
    basic::BasicClient,
    types::{ClientInfo, InitializeResult, ServerCapabilities},
    Client,
};

#[derive(Clone)]
pub enum SecureValue {
    Static(String),
    Env(String),
}

pub struct SecureClient {
    basic: BasicClient,
    secure_values: Arc<RwLock<HashMap<String, SecureValue>>>,
}

pub struct SecureClientBuilder {
    secure_values: HashMap<String, SecureValue>,
}

impl SecureClientBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            secure_values: HashMap::new(),
        }
    }

    /// Add a secure value.
    ///
    /// The key is the name you will use later to retrieve the value.
    /// The `value` may be either `SecureValue::Static` (in which case the stored
    /// value is returned) or `SecureValue::Env` (in which case the stored value
    /// is interpreted as an environment variable name and the environment variable’s
    /// value is returned).
    pub fn with_secure_value(mut self, key: impl Into<String>, value: SecureValue) -> Self {
        self.secure_values.insert(key.into(), value);
        self
    }

    /// Build the `SecureClient`.
    pub fn build(self) -> SecureClient {
        SecureClient {
            basic: BasicClient::new(),
            secure_values: Arc::new(RwLock::new(self.secure_values)),
        }
    }
}

impl SecureClient {
    /// Add a secure value to the client's hash map.
    pub async fn add_secure_value(&self, key: impl Into<String>, value: SecureValue) {
        self.secure_values.write().await.insert(key.into(), value);
    }

    /// Retrieve a secure value from the client.
    ///
    /// If the stored value is a `Static` variant, the stored string is returned.
    /// If the stored value is an `Env` variant, the stored string is used as the key
    /// in `std::env::var` to retrieve the actual value.
    pub async fn get_secure_value(&self, key: &str) -> Result<String, McpError> {
        let map = self.secure_values.read().await;
        let secure_val = map.get(key).ok_or_else(|| {
            println!("Secure value not found for key: {}", key);
            McpError::InvalidRequest(format!("Secure value not found for key: {}", key))
        })?;
        match secure_val {
            SecureValue::Static(val) => Ok(val.clone()),
            SecureValue::Env(env_key) => env::var(env_key).map_err(|e| {
                McpError::InvalidRequest(format!(
                    "Environment variable {} not found: {}",
                    env_key, e
                ))
            }),
        }
    }

    /// Recursively walk through the JSON value. If a JSON string exactly matches
    /// one of the keys in our secure values map, replace it with the corresponding secure value.
    pub async fn apply_secure_replacements(&self, value: &mut Value) -> Result<(), McpError> {
        match value {
            Value::Object(map) => {
                for (_k, v) in map.iter_mut() {
                    if let Value::String(_) = v {
                        if let Ok(replacement) = self.get_secure_value(_k).await {
                            *v = Value::String(replacement);
                        }
                    }
                    Box::pin(self.apply_secure_replacements(v)).await?;
                }
            }
            Value::Array(arr) => {
                for v in arr.iter_mut() {
                    if let Value::Object(_) | Value::Array(_) = v {
                        Box::pin(self.apply_secure_replacements(v)).await?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[async_trait]
impl Client for SecureClient {
    async fn connect<T: Transport + Send + Sync + 'static>(
        &mut self,
        transport: T,
    ) -> Result<ProtocolHandle, McpError> {
        self.basic.connect(transport).await
    }

    async fn initialize(&mut self, client_info: ClientInfo) -> Result<InitializeResult, McpError> {
        self.basic.initialize(client_info).await
    }

    async fn list_resources(
        &self,
        cursor: Option<String>,
    ) -> Result<ListResourcesResponse, McpError> {
        self.basic.list_resources(cursor).await
    }

    async fn read_resource(&self, uri: String) -> Result<ReadResourceResponse, McpError> {
        self.basic.read_resource(uri).await
    }

    async fn subscribe_to_resource(&self, uri: String) -> Result<(), McpError> {
        self.basic.subscribe_to_resource(uri).await
    }

    async fn list_prompts(&self, cursor: Option<String>) -> Result<ListPromptsResponse, McpError> {
        self.basic.list_prompts(cursor).await
    }

    async fn get_prompt(
        &self,
        name: String,
        arguments: Option<Value>,
    ) -> Result<PromptResult, McpError> {
        self.basic.get_prompt(name, arguments).await
    }

    async fn list_tools(&self, cursor: Option<String>) -> Result<ListToolsResponse, McpError> {
        self.basic.list_tools(cursor).await
    }

    async fn call_tool(&self, name: String, mut arguments: Value) -> Result<ToolResult, McpError> {
        self.apply_secure_replacements(&mut arguments).await?;
        self.basic.call_tool(name, arguments).await
    }

    async fn set_log_level(&self, level: String) -> Result<(), McpError> {
        self.basic.set_log_level(level).await
    }

    async fn shutdown(&mut self) -> Result<(), McpError> {
        self.basic.shutdown().await
    }

    async fn get_server_capabilities(&self) -> Option<ServerCapabilities> {
        self.basic.get_server_capabilities().await
    }

    async fn has_capability(&self, capability: &str) -> bool {
        self.basic.has_capability(capability).await
    }

    async fn get_client_info(&self) -> Option<ClientInfo> {
        self.basic.get_client_info().await
    }

    async fn has_client_info(&self) -> bool {
        self.basic.has_client_info().await
    }

    async fn assert_initialized(&self) -> Result<(), McpError> {
        self.basic.assert_initialized().await
    }

    async fn assert_capability(&self, capability: &str) -> Result<(), McpError> {
        self.basic.assert_capability(capability).await
    }

    async fn cleanup_resources(&mut self) -> Result<(), McpError> {
        self.basic.cleanup_resources().await
    }

    async fn wait_for_shutdown(&mut self) -> Result<(), McpError> {
        self.basic.wait_for_shutdown().await
    }
}
