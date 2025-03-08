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
                .request(
                    "tools/list",
                    None,
                    RequestOptions::default().timeout(Duration::from_secs(5)),
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
