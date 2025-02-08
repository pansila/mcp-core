pub mod types;

use crate::{
    error::McpError,
    prompts::{GetPromptRequest, ListPromptsRequest, ListPromptsResponse, PromptResult},
    protocol::{Protocol, ProtocolHandle, ProtocolOptions},
    resource::{
        ListResourcesRequest, ListResourcesResponse, ReadResourceRequest, ReadResourceResponse,
    },
    tools::{CallToolRequest, ListToolsRequest, ListToolsResponse, ToolResult},
    transport::{Transport, TransportCommand},
};
use serde_json::Value;
use std::{
    collections::HashMap,
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::RwLock;
use types::{
    ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, RootsCapabilities,
    SamplingCapabilities, ServerCapabilities,
};

#[derive(Clone)]
pub enum SecureValue {
    Static(String),
    Env(String),
}

pub struct ClientBuilder {
    env: HashMap<String, SecureValue>,
}

impl ClientBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            env: HashMap::new(),
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
        self.env.insert(key.into(), value);
        self
    }

    /// Build the `Client`.
    pub fn build(self) -> Client {
        Client {
            protocol: Protocol::builder(Some(ProtocolOptions {
                enforce_strict_capabilities: true,
            }))
            .build(),
            initialized: Arc::new(RwLock::new(false)),
            client_info: Arc::new(RwLock::new(None)),
            server_capabilities: Arc::new(RwLock::new(None)),
            env: Some(Arc::new(RwLock::new(self.env))),
        }
    }
}

pub struct Client {
    protocol: Protocol,
    initialized: Arc<RwLock<bool>>,
    client_info: Arc<RwLock<Option<ClientInfo>>>,
    server_capabilities: Arc<RwLock<Option<ServerCapabilities>>>,
    env: Option<Arc<RwLock<HashMap<String, SecureValue>>>>,
}

impl Client {
    pub fn new() -> Self {
        Self {
            protocol: Protocol::builder(Some(ProtocolOptions {
                enforce_strict_capabilities: true,
            }))
            .build(),
            initialized: Arc::new(RwLock::new(false)),
            client_info: Arc::new(RwLock::new(None)),
            server_capabilities: Arc::new(RwLock::new(None)),
            env: None,
        }
    }

    pub async fn connect<T: Transport>(
        &mut self,
        transport: T,
    ) -> Result<ProtocolHandle, McpError> {
        let timeout = Duration::from_secs(30);
        match tokio::time::timeout(timeout, self.protocol.connect(transport)).await {
            Ok(result) => result,
            Err(_) => Err(McpError::ConnectionClosed),
        }
    }

    pub async fn initialize(
        &mut self,
        client_info: ClientInfo,
    ) -> Result<InitializeResult, McpError> {
        // Ensure we're not already initialized
        if *self.initialized.read().await {
            return Err(McpError::InvalidRequest(
                "Client already initialized".to_string(),
            ));
        }

        // Prepare initialization parameters
        let params = InitializeParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ClientCapabilities {
                roots: Some(RootsCapabilities { list_changed: true }),
                sampling: Some(SamplingCapabilities {}),
            },
            client_info: client_info.clone(),
        };

        // Send initialize request
        let result: InitializeResult = self
            .protocol
            .request("initialize", Some(params), None)
            .await?;

        // Validate protocol version
        if result.protocol_version != "2024-11-05" {
            return Err(McpError::InvalidRequest(format!(
                "Unsupported protocol version: {}",
                result.protocol_version
            )));
        }

        // Store server capabilities
        *self.server_capabilities.write().await = Some(result.capabilities.clone());

        // Send initialized notification
        self.protocol
            .notification("initialized", Option::<()>::None)
            .await?;

        // Mark as initialized
        *self.initialized.write().await = true;

        // Store client info
        *self.client_info.write().await = Some(client_info);

        Ok(result)
    }

    // Resource methods
    pub async fn list_resources(
        &self,
        cursor: Option<String>,
    ) -> Result<ListResourcesResponse, McpError> {
        self.assert_initialized().await?;
        self.assert_capability("resources").await?;

        self.protocol
            .request(
                "resources/list",
                Some(ListResourcesRequest { cursor }),
                None,
            )
            .await
    }

    pub async fn read_resource(&self, uri: String) -> Result<ReadResourceResponse, McpError> {
        self.assert_initialized().await?;
        self.assert_capability("resources").await?;

        self.protocol
            .request("resources/read", Some(ReadResourceRequest { uri }), None)
            .await
    }

    pub async fn subscribe_to_resource(&self, uri: String) -> Result<(), McpError> {
        self.assert_initialized().await?;
        self.assert_capability("resources").await?;

        self.protocol
            .request("resources/subscribe", Some(uri), None)
            .await
    }

    // Prompt methods
    pub async fn list_prompts(
        &self,
        cursor: Option<String>,
    ) -> Result<ListPromptsResponse, McpError> {
        self.assert_initialized().await?;
        self.assert_capability("prompts").await?;

        self.protocol
            .request("prompts/list", Some(ListPromptsRequest { cursor }), None)
            .await
    }

    pub async fn get_prompt(
        &self,
        name: String,
        arguments: Option<serde_json::Value>,
    ) -> Result<PromptResult, McpError> {
        self.assert_initialized().await?;
        self.assert_capability("prompts").await?;

        self.protocol
            .request(
                "prompts/get",
                Some(GetPromptRequest { name, arguments }),
                None,
            )
            .await
    }

    // Tool methods
    pub async fn list_tools(&self, cursor: Option<String>) -> Result<ListToolsResponse, McpError> {
        self.assert_initialized().await?;
        self.assert_capability("tools").await?;

        self.protocol
            .request("tools/list", Some(ListToolsRequest { cursor }), None)
            .await
    }

    pub async fn call_tool(
        &self,
        name: String,
        arguments: serde_json::Value,
    ) -> Result<ToolResult, McpError> {
        self.assert_initialized().await?;
        self.assert_capability("tools").await?;

        self.protocol
            .request(
                "tools/call",
                Some(CallToolRequest { name, arguments }),
                None,
            )
            .await
    }

    // Logging methods
    pub async fn set_log_level(&self, level: String) -> Result<(), McpError> {
        self.assert_initialized().await?;
        self.assert_capability("logging").await?;

        self.protocol
            .request(
                "logging/setLevel",
                Some(serde_json::json!({ "level": level })),
                None,
            )
            .await
    }

    /// Waits for the server to acknowledge shutdown request
    async fn wait_for_shutdown(&mut self) -> Result<(), McpError> {
        let shutdown_ack = Arc::new(AtomicBool::new(false));

        // Register shutdown handler
        {
            let mut handlers = self.protocol.notification_handlers.write().await;
            let ack = shutdown_ack.clone();
            handlers.insert(
                "shutdown/ack".to_string(),
                Box::new(move |_notification| {
                    let ack = ack.clone();
                    Box::pin(async move {
                        ack.store(true, Ordering::SeqCst);
                        Ok(())
                    })
                }),
            );
        };

        // Send shutdown notification
        self.protocol.notification("shutdown", None::<()>).await?;

        // Wait for acknowledgment
        let mut attempts = 0;
        while !shutdown_ack.load(Ordering::SeqCst) {
            if attempts >= 50 {
                // 5 seconds with 100ms sleep
                return Err(McpError::ShutdownError(
                    "No shutdown acknowledgment received".into(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            attempts += 1;
        }

        // Clean up handler
        self.protocol
            .notification_handlers
            .write()
            .await
            .remove("shutdown/ack");

        // Clean up resources
        self.cleanup_resources().await?;

        Ok(())
    }

    /// Performs graceful shutdown of the client
    pub async fn shutdown(&mut self) -> Result<(), McpError> {
        // Only attempt shutdown if we're initialized
        if !*self.initialized.read().await {
            return Ok(());
        }

        tracing::debug!("Starting client shutdown sequence");

        // Set client state to shutting down
        *self.initialized.write().await = false;

        // Wait for shutdown with timeout
        match tokio::time::timeout(Duration::from_secs(5), self.wait_for_shutdown()).await {
            Ok(result) => {
                tracing::debug!("Client shutdown completed successfully");
                result
            }
            Err(_) => {
                tracing::warn!("Client shutdown timed out");
                // Force cleanup on timeout
                self.cleanup_resources().await?;
                Err(McpError::ShutdownTimeout)
            }
        }
    }

    /// Cleans up client resources
    async fn cleanup_resources(&mut self) -> Result<(), McpError> {
        tracing::debug!("Cleaning up client resources");

        // Close transport
        if let Some(cmd_tx) = &self.protocol.cmd_tx {
            let _ = cmd_tx.send(TransportCommand::Close).await;
            self.protocol.cmd_tx = None;
        }

        // Clear handlers
        self.protocol.notification_handlers.write().await.clear();
        self.protocol.request_handlers.write().await.clear();
        self.protocol.response_handlers.write().await.clear();
        self.protocol.progress_handlers.write().await.clear();

        // Clear capabilities
        *self.server_capabilities.write().await = None;

        Ok(())
    }

    pub async fn assert_initialized(&self) -> Result<(), McpError> {
        if !*self.initialized.read().await {
            return Err(McpError::InvalidRequest(
                "Client not initialized".to_string(),
            ));
        }
        Ok(())
    }

    async fn assert_capability(&self, capability: &str) -> Result<(), McpError> {
        let caps = self.server_capabilities.read().await;
        let caps = caps
            .as_ref()
            .ok_or_else(|| McpError::InvalidRequest("No server capabilities".to_string()))?;

        let has_capability = match capability {
            "logging" => caps.logging.is_some(),
            "prompts" => caps.prompts.is_some(),
            "resources" => caps.resources.is_some(),
            "tools" => caps.tools.is_some(),
            _ => false,
        };

        if !has_capability {
            return Err(McpError::CapabilityNotSupported(capability.to_string()));
        }

        Ok(())
    }

    pub async fn get_server_capabilities(&self) -> Option<ServerCapabilities> {
        self.server_capabilities.read().await.clone()
    }

    // Helper method to check if server supports a capability
    pub async fn has_capability(&self, capability: &str) -> bool {
        if let Some(caps) = self.get_server_capabilities().await {
            match capability {
                "logging" => caps.logging.is_some(),
                "prompts" => caps.prompts.is_some(),
                "resources" => caps.resources.is_some(),
                "tools" => caps.tools.is_some(),
                _ => false,
            }
        } else {
            false
        }
    }

    pub async fn get_client_info(&self) -> Option<ClientInfo> {
        self.client_info.read().await.clone()
    }

    pub async fn has_client_info(&self) -> bool {
        self.get_client_info().await.is_some()
    }

    /// Add a secure value to the client's hash map.
    pub async fn add_secure_value(&self, key: impl Into<String>, value: SecureValue) {
        if let Some(env) = &self.env {
            env.write().await.insert(key.into(), value);
        }
    }

    /// Retrieve a secure value from the client.
    ///
    /// If the stored value is a `Static` variant, the stored string is returned.
    /// If the stored value is an `Env` variant, the stored string is used as the key
    /// in `std::env::var` to retrieve the actual value.
    pub async fn get_secure_value(&self, key: &str) -> Result<String, McpError> {
        if let Some(env) = &self.env {
            let map = env.read().await;
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
        } else {
            Err(McpError::InvalidRequest(
                "Secure values not initialized".to_string(),
            ))
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
