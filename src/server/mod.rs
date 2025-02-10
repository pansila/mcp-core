use config::ServerConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio::sync::RwLock;
use tracing::info;

use crate::prompts::{GetPromptRequest, ListPromptsRequest, PromptCapabilities, PromptManager};
use crate::tools::{ToolCapabilities, ToolManager};
use crate::{
    client::types::ServerCapabilities,
    error::McpError,
    logging::{LoggingCapabilities, LoggingManager, SetLevelRequest},
    protocol::types::*,
    protocol::{
        BasicRequestHandler, JsonRpcNotification, Protocol, ProtocolBuilder, ProtocolOptions,
        RequestHandler,
    },
    resource::{ListResourcesRequest, ReadResourceRequest, ResourceCapabilities, ResourceManager},
    tools::{CallToolRequest, ListToolsRequest},
    transport::{stdio::StdioTransport, SseServerTransport, Transport},
};
use tokio::sync::mpsc;

pub mod config;

// Add initialization types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: ClientInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: InitializeServerInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    pub roots: Option<RootsCapabilities>,
    pub sampling: Option<SamplingCapabilities>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootsCapabilities {
    pub list_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SamplingCapabilities {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

// Add server state enum
#[derive(Debug, Clone, Copy, PartialEq)]
enum ServerState {
    Created,
    Initializing,
    Running,
    ShuttingDown,
}

pub struct McpServer<H>
where
    H: RequestHandler + Send + Sync + 'static,
{
    pub handler: Arc<H>,
    pub config: ServerConfig,
    pub resource_manager: Arc<ResourceManager>,
    pub tool_manager: Arc<ToolManager>,
    pub prompt_manager: Arc<PromptManager>,
    pub logging_manager: Arc<tokio::sync::Mutex<LoggingManager>>,
    notification_tx: mpsc::Sender<JsonRpcNotification>,
    notification_rx: Option<mpsc::Receiver<JsonRpcNotification>>, // Make this Option
    state: Arc<(watch::Sender<ServerState>, watch::Receiver<ServerState>)>,
    supported_versions: Vec<String>,
    client_capabilities: Arc<RwLock<Option<ClientCapabilities>>>,
}

impl<H> McpServer<H>
where
    H: RequestHandler + Send + Sync + 'static,
{
    pub fn new(config: ServerConfig, handler: H) -> Self {
        let (notification_tx, notification_rx) = mpsc::channel(100);
        let (state_tx, state_rx) = watch::channel(ServerState::Created);

        Self {
            handler: Arc::new(handler),
            config: config.clone(),
            resource_manager: Arc::new(ResourceManager::new(ResourceCapabilities {
                subscribe: false,
                list_changed: false,
            })),
            tool_manager: Arc::new(ToolManager::new(ToolCapabilities {
                list_changed: false,
            })),
            prompt_manager: Arc::new(PromptManager::new(PromptCapabilities {
                list_changed: false,
            })),
            logging_manager: Arc::new(tokio::sync::Mutex::new(LoggingManager::new())),
            notification_tx,
            notification_rx: Some(notification_rx),
            state: Arc::new((state_tx, state_rx)),
            supported_versions: vec!["1.0".to_string()],
            client_capabilities: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn process_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, McpError> {
        self.handler.handle_request(method, params).await
    }

    pub async fn run_transport<T: Transport>(&mut self, transport: T) -> Result<(), McpError> {
        self.notification_rx.take().ok_or_else(|| {
            McpError::InternalError("Notification receiver already taken".to_string())
        })?;

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            while *state.1.borrow() != ServerState::ShuttingDown {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let _ = shutdown_tx.send(()).await;
        });

        let resource_manager = Arc::clone(&self.resource_manager);
        let tool_manager = Arc::clone(&self.tool_manager);
        let prompt_manager = Arc::clone(&self.prompt_manager);

        let mut protocol = Protocol::builder(Some(ProtocolOptions {
            enforce_strict_capabilities: false,
        }))
        .with_request_handler(
            "initialize",
            Box::new(|req, _extra| {
                Box::pin(async move {
                    let params: InitializeParams =
                        serde_json::from_value(req.params.unwrap_or_default())
                            .map_err(|_| McpError::InvalidParams)?;

                    // TODO: Verify client params

                    let result = InitializeResult {
                        protocol_version: "2024-11-05".to_string(),
                        capabilities: ServerCapabilities {
                            logging: Some(LoggingCapabilities {}),
                            prompts: Some(PromptCapabilities {
                                list_changed: false,
                            }),
                            resources: Some(ResourceCapabilities {
                                subscribe: false,
                                list_changed: false,
                            }),
                            tools: Some(ToolCapabilities {
                                list_changed: false,
                            }),
                        },
                        server_info: InitializeServerInfo {
                            name: "test-server".to_string(),
                            version: "1.0.0".to_string(),
                        },
                    };
                    Ok(serde_json::to_value(result).unwrap())
                })
            }),
        )
        .with_request_handler(
            "resources/list",
            Box::new({
                // Closure captures a clone for later use.
                let rm = resource_manager.clone();
                move |req, _extra| {
                    let rm = rm.clone();
                    Box::pin(async move {
                        let params: ListResourcesRequest = req
                            .params
                            .map(serde_json::from_value)
                            .transpose()
                            .map_err(|_| McpError::InvalidParams)?
                            .unwrap_or_default();
                        let resources_list = rm.list_resources(params.cursor).await?;
                        Ok(serde_json::to_value(resources_list).unwrap())
                    })
                }
            }),
        )
        .with_request_handler(
            "resources/read",
            Box::new({
                let rm = resource_manager.clone();
                move |req, _extra| {
                    let rm = rm.clone();
                    Box::pin(async move {
                        let params: ReadResourceRequest =
                            serde_json::from_value(req.params.unwrap_or_default())
                                .map_err(|_| McpError::InvalidParams)?;
                        let resource = rm.read_resource(&params.uri).await?;
                        Ok(serde_json::to_value(resource).unwrap())
                    })
                }
            }),
        )
        .with_request_handler(
            "resources/templates/list",
            Box::new({
                let rm = resource_manager.clone();
                move |_req, _extra| {
                    let rm = rm.clone();
                    Box::pin(async move {
                        let templates_list = rm.list_templates().await?;
                        Ok(serde_json::to_value(templates_list).unwrap())
                    })
                }
            }),
        )
        .with_request_handler(
            "tools/list",
            Box::new({
                let tm = tool_manager.clone();
                move |req, _extra| {
                    let tm = tm.clone();
                    Box::pin(async move {
                        let params: ListToolsRequest = req
                            .params
                            .map(serde_json::from_value)
                            .transpose()
                            .map_err(|_| McpError::InvalidParams)?
                            .unwrap_or_default();
                        let tools_list = tm.list_tools(params.cursor).await?;
                        Ok(serde_json::to_value(tools_list).unwrap())
                    })
                }
            }),
        )
        .with_request_handler(
            "tools/call",
            Box::new({
                let tm = tool_manager.clone();
                move |req, _extra| {
                    let tm = tm.clone();
                    Box::pin(async move {
                        let params: CallToolRequest =
                            serde_json::from_value(req.params.unwrap_or_default())
                                .map_err(|_| McpError::InvalidParams)?;
                        let result = tm.call_tool(&params.name, params.arguments).await?;
                        Ok(serde_json::to_value(result).unwrap())
                    })
                }
            }),
        )
        .with_request_handler(
            "prompts/list",
            Box::new({
                let pm = prompt_manager.clone();
                move |req, _extra| {
                    let pm = pm.clone();
                    Box::pin(async move {
                        let params: ListPromptsRequest = req
                            .params
                            .map(serde_json::from_value)
                            .transpose()
                            .map_err(|_| McpError::InvalidParams)?
                            .unwrap_or_default();
                        let prompts_list = pm.list_prompts(params.cursor).await?;
                        Ok(serde_json::to_value(prompts_list).unwrap())
                    })
                }
            }),
        )
        .with_request_handler(
            "prompts/get",
            Box::new({
                let pm = prompt_manager.clone();
                move |req, _extra| {
                    let pm = pm.clone();
                    Box::pin(async move {
                        let params: GetPromptRequest =
                            serde_json::from_value(req.params.unwrap_or_default())
                                .map_err(|_| McpError::InvalidParams)?;
                        let prompt = pm.get_prompt(&params.name, params.arguments).await?;
                        Ok(serde_json::to_value(prompt).unwrap())
                    })
                }
            }),
        )
        .build();

        let protocol_handle = protocol.connect(transport).await?;

        shutdown_rx.recv().await;

        protocol_handle.close().await?;
        Ok(())
    }
}
