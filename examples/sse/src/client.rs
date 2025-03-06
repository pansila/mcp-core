use mcp_core::{
    client::ClientBuilder,
    transport::{ClientSseTransportBuilder, Transport},
    types::Implementation,
};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stderr)
        .init();

    let transport = ClientSseTransportBuilder::new("http://127.0.0.1:8080".to_string()).build();
    transport.open().await?;

    let client = ClientBuilder::new(transport).use_strict().build();

    let init_res = client
        .initialize(Implementation {
            name: "mcp-client".to_string(),
            version: "0.1.0".to_string(),
        })
        .await?;
    println!("Initialized: {:?}", init_res);

    let response = client.list_tools(None, None).await?;
    println!("Tools: {:?}", response);

    Ok(())
}
