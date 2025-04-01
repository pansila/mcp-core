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

## Project Goals
Combine efforts with [Offical MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk). The offical SDK repo is new and collaborations are in works to bring these features to the adopted platform.
- **Efficiency & Scalability**
  - Handles many concurrent connections with low overhead.
  - Scales easily across multiple nodes.
- **Security**
  - Strong authentication and authorization.
  - Built-in rate limiting and quota management.
- **Rust Advantages**
  - High performance and predictable latency.
  - Memory safety with no runtime overhead.

## Installation

Use the `cargo add` command to automatically add it to your `Cargo.toml`
```bash
cargo add mcp-core
```
Or add `mcp-core` to your `Cargo.toml` dependencies directly
```toml
[dependencies]
mcp-core = "0.1.43"
```

## Server Implementation
Easily start your own local SSE MCP Servers with tooling capabilities. To use SSE functionality, make sure to enable the "http" feature in your Cargo.toml `mcp-core = { version = "0.1.42", features = ["sse"] }`
```rs
mod echo;
use anyhow::Result;
use clap::{Parser, ValueEnum};
use echo::*;
use mcp_core::{
    server::Server,
    transport::{ServerSseTransport, ServerStdioTransport},
    types::ServerCapabilities,
};
use serde_json::json;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Transport type to use
    #[arg(value_enum, default_value_t = TransportType::Sse)]
    transport: TransportType,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum TransportType {
    Stdio,
    Sse,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        // needs to be stderr due to stdio transport
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    let server_protocol = Server::builder("echo".to_string(), "1.0".to_string())
        .capabilities(ServerCapabilities {
            tools: Some(json!({
                "listChanged": false,
            })),
            ..Default::default()
        })
        .register_tool(EchoTool::tool(), EchoTool::call().await)
        .build();

    match cli.transport {
        TransportType::Stdio => {
            let transport = ServerStdioTransport::new(server_protocol);
            Server::start(transport).await
        }
        TransportType::Sse => {
            let transport = ServerSseTransport::new("127.0.0.1".to_string(), 3000, server_protocol);
            Server::start(transport).await
        }
    }
}
```

## Creating MCP Tools
There are two ways to create tools in MCP Core: using macros (recommended) or manually implementing the tool trait.

### Using Macros (Recommended)
The easiest way to create a tool is using the `mcp-core-macros` crate. First, add it to your dependencies:
```toml
[dependencies]
mcp-core-macros = "0.1.11"
```

Then create your tool using the `#[tool]` macro:
```rust
use mcp_core::{tool_text_content, types::ToolResponseContent};
use mcp_core_macros::tool;
use anyhow::Result;

#[tool(
    name = "echo",
    description = "Echo back the message you send",
    params(message = "The message to echo back")
)]
async fn echo_tool(message: String) -> Result<ToolResponseContent> {
    Ok(tool_text_content!(message))
}
```

The macro automatically generates all the necessary boilerplate code for your tool. You can then register it with your server:

```rust
let server_protocol = Server::builder("echo".to_string(), "1.0".to_string())
    .capabilities(ServerCapabilities {
        tools: Some(json!({
            "listChanged": false,
        })),
        ..Default::default()
    })
    .register_tool(EchoTool::tool(), EchoTool::call())
    .build();
```

### Tool Parameters
Tools can have various parameter types that are automatically deserialized from the client's JSON input:
- Basic types (String, f64, bool)
- Optional types (Option<T>)

For example:
```rust
#[derive(Deserialize)]
struct ComplexParam {
    field1: String,
    field2: f64,
}

#[tool(
    name = "complex_tool",
    description = "A tool with complex parameters",
    params(
        text = "A text parameter",
        number = "An optional number parameter",
    )
)]
async fn complex_tool(
    text: String, 
    number: Option<f64>,
) -> Result<ToolResponseContent> {
    // Tool implementation
}
```

## SSE Client Connection
Connect to an SSE MCP Server using the `ClientSseTransport`. Here is an example of connecting to one and listing the tools from that server.
```rs
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use mcp_core::{
    client::ClientBuilder,
    protocol::RequestOptions,
    transport::{ClientSseTransportBuilder, ClientStdioTransport},
    types::{ClientCapabilities, Implementation},
};
use serde_json::json;
use tracing::info;
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Transport type to use
    #[arg(value_enum, default_value_t = TransportType::Sse)]
    transport: TransportType,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum TransportType {
    Stdio,
    Sse,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    let response = match cli.transport {
        TransportType::Stdio => {
            // Build the server first
            // cargo build --bin echo_server
            let transport = ClientStdioTransport::new("./target/debug/echo_server", &[])?;
            let client = ClientBuilder::new(transport.clone()).build();
            tokio::time::sleep(Duration::from_millis(100)).await;
            client.open().await?;

            client
                .initialize(
                    Implementation {
                        name: "echo".to_string(),
                        version: "1.0".to_string(),
                    },
                    ClientCapabilities::default(),
                )
                .await?;

            client
                .call_tool(
                    "echo",
                    Some(json!({
                        "message": "Hello, world!"
                    })),
                )
                .await?
        }
        TransportType::Sse => {
            let client = ClientBuilder::new(
                ClientSseTransportBuilder::new("http://localhost:3000/sse".to_string()).build(),
            )
            .build();
            client.open().await?;

            client
                .initialize(
                    Implementation {
                        name: "echo".to_string(),
                        version: "1.0".to_string(),
                    },
                    ClientCapabilities::default(),
                )
                .await?;

            client
                .call_tool(
                    "echo",
                    Some(json!({
                        "message": "Hello, world!"
                    })),
                )
                .await?
        }
    };
    info!("response: {:?}", response);
    Ok(())
}
```

### Setting `SecureValues` to your SSE MCP Client
Have API Keys or Secrets needed to be passed to MCP Tool Calls, but you don't want to pass this information to the LLM you are prompting? Use `mcp_core::client::SecureValue`!
```rs
ClientBuilder::new(
    ClientSseTransportBuilder::new("http://localhost:3000/sse".to_string()).build(),
)
.with_secure_value(
    "discord_token",
    mcp_core::client::SecureValue::Static(discord_token),
)
.with_secure_value(
    "anthropic_api_key",
    mcp_core::client::SecureValue::Env("ANTHROPIC_API_KEY".to_string()),
)
.use_strict()
.build()
```
#### mcp_core::client::SecureValue::Static
Automatically have **MCP Tool Call Parameters** be replaced by the string value set to it.
#### mcp_core::client::SecureValue::Env
Automatically have **MCP Tool Call Parameters** be replaced by the value in your `.env` from the string set to it.