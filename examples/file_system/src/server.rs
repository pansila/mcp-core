mod tools;
use anyhow::Result;
use clap::{Parser, ValueEnum};
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

    let server_protocol = Server::builder("pingpong".to_string(), "1.0".to_string())
        .capabilities(ServerCapabilities {
            tools: Some(json!({
                "listChanged": false,
            })),
            ..Default::default()
        })
        .register_tool(
            tools::GetFileInfoTool::tool(),
            tools::GetFileInfoTool::call().await,
        )
        .register_tool(
            tools::ListDirectoryTool::tool(),
            tools::ListDirectoryTool::call().await,
        )
        .register_tool(
            tools::ReadFileTool::tool(),
            tools::ReadFileTool::call().await,
        )
        .register_tool(
            tools::SearchFilesTool::tool(),
            tools::SearchFilesTool::call().await,
        )
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
