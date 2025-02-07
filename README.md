<p align="center">
    <img src="imgs/mcp_logo.png" alt="mcp_logo" style="width: 15%; margin-right:3%;" />
    <img src="imgs/plus.svg" alt="plus_svg" style="width: 10%; margin-bottom: 2%;" />
    <img src="imgs/rust_logo.png" alt="rust_logo" style="width: 15%; margin-left:3%;" />
</p>
<p align="center">
<h1 align="center">MCP Core</h1>
<p align="center">
A Rust library implementing the <a href="https://modelcontextprotocol.io/introduction">Modern Context Protocol (MCP)</a>
</p>
<p align="center">
<a href="https://github.com/stevohuncho/mcp-core"><img src="https://img.shields.io/github/stars/stevohuncho/mcp-core?style=social" alt="stars" /></a>
&nbsp;
<a href="https://crates.io/crates/mcp-core"><img src="https://img.shields.io/crates/v/mcp-core" alt="Crates.io" /></a>
&nbsp;
</p>

## Installation

Use the `cargo add` command to automatically add it to your `Cargo.toml`
```bash
cargo add mcp_core
```
Or add `mcp_core` to your `Cargo.toml` dependencies directly
```toml
[dependencies]
mcp_core = "0.0.1"
```

## Quickstart

### MCP Servers

You can run your own **STDIO** + **SSE** MCP Server simply by running the `just` commands:
```bash
just server
```
```bash
just sse-server
```

### MCP Client

Run an **SSE** Client via the `just` command below:
```bash
just sse-client
```

## Examples

### SSE Server
A working example of implementing `mcp_core` into your project running an SSE Server on `http://127.0.0.1:8080`.
```rust
use mcp_core::{
    logging::McpSubscriber,
    protocol::BasicRequestHandler,
    server::{
        config::{ServerConfig, ServerSettings, TransportType},
        McpServer,
    },
    tools::calculator::CalculatorTool,
};
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    let config = ServerConfig {
        server: ServerSettings {
            transport: TransportType::Sse,
            version: "1".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8080,
            max_connections: 100,
            timeout_ms: 30000,
            ..ServerConfig::default().server
        },
        ..ServerConfig::default()
    };

    let handler =
        BasicRequestHandler::new(config.server.name.clone(), config.server.version.clone());

    let mut server = McpServer::new(config, handler);

    let mcp_subscriber = McpSubscriber::new(Arc::clone(&server.logging_manager));

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_thread_ids(true)
                .with_line_number(true),
        )
        .with(mcp_subscriber)
        .init();

    server
        .tool_manager
        .register_tool(Arc::new(CalculatorTool::new()))
        .await;

    tracing::info!("Enabled capabilities:");
    tracing::info!("  Resources:");
    tracing::info!(
        "    - subscribe: {}",
        server.resource_manager.capabilities.subscribe
    );
    tracing::info!(
        "    - listChanged: {}",
        server.resource_manager.capabilities.list_changed
    );
    tracing::info!("  Tools:");
    tracing::info!(
        "    - listChanged: {}",
        server.tool_manager.capabilities.list_changed
    );
    tracing::info!("  Prompts:");
    tracing::info!(
        "    - listChanged: {}",
        server.prompt_manager.capabilities.list_changed
    );

    tokio::select! {
        result = server.run_sse_transport() => {
            if let Err(e) = result {
                tracing::error!("Server error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Shutting down server...");
        }
    }
}
```

### SSE Client
A working example of implementing `mcp_core` into your project running an SSE Client connected to `http://127.0.0.1:8080`.
```rust
use mcp_core::{
    client::{Client, ClientInfo},
    error::McpError,
    transport::sse::SseTransport,
};

#[tokio::main]
async fn main() -> Result<(), McpError> {
    tracing_subscriber::fmt().init();

    let mut mcp_client = Client::new();

    let server_url = "http://127.0.0.1".to_string();
    // Parse server URL to get host and port
    let url = url::Url::parse(&server_url).unwrap();
    let host = url.host_str().unwrap_or("127.0.0.1").to_string();
    let port = url.port().unwrap_or(8080);

    let transport = SseTransport::new_client(host, port, 32);
    mcp_client.connect(transport).await?;

    tracing::debug!("Sending initialize request...");
    let init_result = tokio::time::timeout(
        std::time::Duration::from_secs(30), // Increased from 5 to 30 seconds
        mcp_client.initialize(ClientInfo {
            name: "mcp-cli".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }),
    )
    .await
    .map_err(|_| {
        tracing::error!("Initialize request timed out");
        McpError::RequestTimeout
    })??;

    tracing::info!(
        "Successfully initialized. Server info: {:?}",
        init_result.server_info
    );

    let tools = mcp_client.list_tools(None).await.map_err(|e| {
        tracing::error!("Failed to list tools: {}", e);
        e
    })?;

    tracing::info!("List of tools: {:?}", tools);

    Ok(())
}
```