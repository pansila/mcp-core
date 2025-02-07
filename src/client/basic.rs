use async_trait::async_trait;
use serde_json::Value;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::RwLock;
use tokio::time;

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

use super::{
    types::{
        ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, RootsCapabilities,
        SamplingCapabilities, ServerCapabilities,
    },
    Client,
};

pub struct BasicClient {
    protocol: Protocol,
    initialized: Arc<RwLock<bool>>,
    client_info: Arc<RwLock<Option<ClientInfo>>>,
    server_capabilities: Arc<RwLock<Option<ServerCapabilities>>>,
}

impl BasicClient {
    pub fn new() -> Self {
        Self {
            protocol: Protocol::builder(Some(ProtocolOptions {
                enforce_strict_capabilities: true,
            }))
            .build(),
            initialized: Arc::new(RwLock::new(false)),
            client_info: Arc::new(RwLock::new(None)),
            server_capabilities: Arc::new(RwLock::new(None)),
        }
    }
}

#[async_trait]
impl Client for BasicClient {
    async fn connect<T: Transport + Send + Sync + 'static>(
        &mut self,
        transport: T,
    ) -> Result<ProtocolHandle, McpError> {
        let timeout = Duration::from_secs(30);
        match tokio::time::timeout(timeout, self.protocol.connect(transport)).await {
            Ok(result) => result,
            Err(_) => Err(McpError::ConnectionClosed),
        }
    }

    async fn initialize(&mut self, client_info: ClientInfo) -> Result<InitializeResult, McpError> {
        if *self.initialized.read().await {
            return Err(McpError::InvalidRequest(
                "Client already initialized".to_string(),
            ));
        }
        let params = InitializeParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ClientCapabilities {
                roots: Some(RootsCapabilities { list_changed: true }),
                sampling: Some(SamplingCapabilities {}),
            },
            client_info: client_info.clone(),
        };
        let result: InitializeResult = self
            .protocol
            .request("initialize", Some(params), None)
            .await?;
        if result.protocol_version != "2024-11-05" {
            return Err(McpError::InvalidRequest(format!(
                "Unsupported protocol version: {}",
                result.protocol_version
            )));
        }
        *self.server_capabilities.write().await = Some(result.capabilities.clone());
        self.protocol
            .notification("initialized", Option::<()>::None)
            .await?;
        *self.initialized.write().await = true;
        *self.client_info.write().await = Some(client_info);
        Ok(result)
    }

    async fn list_resources(
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

    async fn read_resource(&self, uri: String) -> Result<ReadResourceResponse, McpError> {
        self.assert_initialized().await?;
        self.assert_capability("resources").await?;
        self.protocol
            .request("resources/read", Some(ReadResourceRequest { uri }), None)
            .await
    }

    async fn subscribe_to_resource(&self, uri: String) -> Result<(), McpError> {
        self.assert_initialized().await?;
        self.assert_capability("resources").await?;
        self.protocol
            .request("resources/subscribe", Some(uri), None)
            .await
    }

    async fn list_prompts(&self, cursor: Option<String>) -> Result<ListPromptsResponse, McpError> {
        self.assert_initialized().await?;
        self.assert_capability("prompts").await?;
        self.protocol
            .request("prompts/list", Some(ListPromptsRequest { cursor }), None)
            .await
    }

    async fn get_prompt(
        &self,
        name: String,
        arguments: Option<Value>,
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

    async fn list_tools(&self, cursor: Option<String>) -> Result<ListToolsResponse, McpError> {
        self.assert_initialized().await?;
        self.assert_capability("tools").await?;
        self.protocol
            .request("tools/list", Some(ListToolsRequest { cursor }), None)
            .await
    }

    async fn call_tool(&self, name: String, arguments: Value) -> Result<ToolResult, McpError> {
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

    async fn set_log_level(&self, level: String) -> Result<(), McpError> {
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

    async fn shutdown(&mut self) -> Result<(), McpError> {
        if !*self.initialized.read().await {
            return Ok(());
        }
        tracing::debug!("Starting client shutdown sequence");
        *self.initialized.write().await = false;
        match tokio::time::timeout(Duration::from_secs(5), self.wait_for_shutdown()).await {
            Ok(result) => {
                tracing::debug!("Client shutdown completed successfully");
                result
            }
            Err(_) => {
                tracing::warn!("Client shutdown timed out");
                self.cleanup_resources().await?;
                Err(McpError::ShutdownTimeout)
            }
        }
    }

    async fn get_server_capabilities(&self) -> Option<ServerCapabilities> {
        self.server_capabilities.read().await.clone()
    }

    async fn has_capability(&self, capability: &str) -> bool {
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

    async fn get_client_info(&self) -> Option<ClientInfo> {
        self.client_info.read().await.clone()
    }

    async fn has_client_info(&self) -> bool {
        self.get_client_info().await.is_some()
    }

    /// Ensure that the client is initialized.
    async fn assert_initialized(&self) -> Result<(), McpError> {
        if !*self.initialized.read().await {
            return Err(McpError::InvalidRequest(
                "Client not initialized".to_string(),
            ));
        }
        Ok(())
    }

    /// Ensure that the server has advertised a given capability.
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

    /// Clean up internal resources (handlers, transports, etc.).
    async fn cleanup_resources(&mut self) -> Result<(), McpError> {
        tracing::debug!("Cleaning up client resources");
        if let Some(cmd_tx) = &self.protocol.cmd_tx {
            let _ = cmd_tx.send(TransportCommand::Close).await;
            self.protocol.cmd_tx = None;
        }
        self.protocol.notification_handlers.write().await.clear();
        self.protocol.request_handlers.write().await.clear();
        self.protocol.response_handlers.write().await.clear();
        self.protocol.progress_handlers.write().await.clear();
        *self.server_capabilities.write().await = None;
        Ok(())
    }

    /// Wait for the server to acknowledge a shutdown notification.
    async fn wait_for_shutdown(&mut self) -> Result<(), McpError> {
        let shutdown_ack = Arc::new(AtomicBool::new(false));
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
        }
        self.protocol.notification("shutdown", None::<()>).await?;
        let mut attempts = 0;
        while !shutdown_ack.load(Ordering::SeqCst) {
            if attempts >= 50 {
                return Err(McpError::ShutdownError(
                    "No shutdown acknowledgment received".into(),
                ));
            }
            time::sleep(Duration::from_millis(100)).await;
            attempts += 1;
        }
        self.protocol
            .notification_handlers
            .write()
            .await
            .remove("shutdown/ack");
        self.cleanup_resources().await?;
        Ok(())
    }
}
