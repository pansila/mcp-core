use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use crate::{
    protocol::Protocol,
    tools::{ToolHandler, Tools},
    types::{CallToolRequest, CallToolResponse, ListRequest, Tool, ToolsListResponse},
};

use super::{
    protocol::ProtocolBuilder,
    transport::Transport,
    types::{
        ClientCapabilities, Implementation, InitializeRequest, InitializeResponse,
        ServerCapabilities, LATEST_PROTOCOL_VERSION,
    },
};
use anyhow::Result;
use std::future::Future;
use std::pin::Pin;

#[derive(Clone)]
pub struct ClientConnection {
    client_capabilities: Option<ClientCapabilities>,
    client_info: Option<Implementation>,
    initialized: bool,
}

#[derive(Clone)]
pub struct Server;

impl Server {
    pub fn builder(name: String, version: String) -> ServerProtocolBuilder {
        ServerProtocolBuilder::new(name, version)
    }

    pub async fn start<T: Transport>(transport: T) -> Result<()> {
        transport.open().await
    }
}

pub struct ServerProtocolBuilder {
    protocol_builder: ProtocolBuilder,
    server_info: Implementation,
    capabilities: ServerCapabilities,
    tools: HashMap<String, ToolHandler>,
    client_connection: Arc<RwLock<ClientConnection>>,
}

impl ServerProtocolBuilder {
    pub fn new(name: String, version: String) -> Self {
        ServerProtocolBuilder {
            protocol_builder: ProtocolBuilder::new(),
            server_info: Implementation { name, version },
            capabilities: ServerCapabilities::default(),
            tools: HashMap::new(),
            client_connection: Arc::new(RwLock::new(ClientConnection {
                client_capabilities: None,
                client_info: None,
                initialized: false,
            })),
        }
    }

    pub fn capabilities(mut self, capabilities: ServerCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn register_tool(
        mut self,
        tool: Tool,
        f: impl Fn(CallToolRequest) -> Pin<Box<dyn Future<Output = CallToolResponse> + Send>>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.tools.insert(
            tool.name.clone(),
            ToolHandler {
                tool,
                f: Box::new(f),
            },
        );
        self
    }

    // Helper function for initialize handler
    fn handle_init(
        state: Arc<RwLock<ClientConnection>>,
        server_info: Implementation,
        capabilities: ServerCapabilities,
    ) -> impl Fn(
        InitializeRequest,
    )
        -> Pin<Box<dyn std::future::Future<Output = Result<InitializeResponse>> + Send>> {
        move |req| {
            let state = state.clone();
            let server_info = server_info.clone();
            let capabilities = capabilities.clone();

            Box::pin(async move {
                let mut state = state
                    .write()
                    .map_err(|_| anyhow::anyhow!("Lock poisoned"))?;
                state.client_capabilities = Some(req.capabilities);
                state.client_info = Some(req.client_info);

                Ok(InitializeResponse {
                    protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
                    capabilities,
                    server_info,
                })
            })
        }
    }

    // Helper function for initialized handler
    fn handle_initialized(
        state: Arc<RwLock<ClientConnection>>,
    ) -> impl Fn(()) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>> {
        move |_| {
            let state = state.clone();
            Box::pin(async move {
                let mut state = state
                    .write()
                    .map_err(|_| anyhow::anyhow!("Lock poisoned"))?;
                state.initialized = true;
                Ok(())
            })
        }
    }

    pub fn get_client_capabilities(&self) -> Option<ClientCapabilities> {
        self.client_connection
            .read()
            .ok()?
            .client_capabilities
            .clone()
    }

    pub fn get_client_info(&self) -> Option<Implementation> {
        self.client_connection.read().ok()?.client_info.clone()
    }

    pub fn is_initialized(&self) -> bool {
        self.client_connection
            .read()
            .ok()
            .map(|client_connection| client_connection.initialized)
            .unwrap_or(false)
    }

    pub fn build(self) -> Protocol {
        let tools = Arc::new(Tools::new(self.tools));
        let tools_clone = tools.clone();
        let tools_list = tools.clone();
        let tools_call = tools_clone.clone();

        let conn_for_list = self.client_connection.clone();
        let conn_for_call = self.client_connection.clone();

        self.protocol_builder
            .request_handler(
                "initialize",
                Self::handle_init(
                    self.client_connection.clone(),
                    self.server_info,
                    self.capabilities,
                ),
            )
            .notification_handler(
                "notifications/initialized",
                Self::handle_initialized(self.client_connection.clone()),
            )
            .request_handler("tools/list", move |_req: ListRequest| {
                let tools = tools_list.clone();
                let conn = conn_for_list.clone();

                Box::pin(async move {
                    let client_state = conn.read().map_err(|_| anyhow::anyhow!("Lock poisoned"))?;

                    if !client_state.initialized {
                        return Err(anyhow::anyhow!(
                            "Client must be initialized before using tools/list"
                        ));
                    }

                    Ok(ToolsListResponse {
                        tools: tools.list_tools(),
                        next_cursor: None,
                        meta: None,
                    })
                })
            })
            .request_handler("tools/call", move |req: CallToolRequest| {
                let tools = tools_call.clone();
                let conn = conn_for_call.clone();

                Box::pin(async move {
                    {
                        // Check if client is initialized
                        let client_state =
                            conn.read().map_err(|_| anyhow::anyhow!("Lock poisoned"))?;

                        if !client_state.initialized {
                            return Err(anyhow::anyhow!(
                                "Client must be initialized before using tools/call"
                            ));
                        }
                    }

                    tools.call_tool(req).await
                })
            })
            .build()
    }
}
