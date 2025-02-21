use dotenv::dotenv;
use mcp_core::{
    client::{ClientBuilder, SecureValue},
    transport::{ClientSseTransportBuilder, Transport},
    types::Implementation,
};
use serde_json::json;

#[tokio::test]
async fn test_sse_client_list_tools() -> Result<(), anyhow::Error> {
    let transport =
        ClientSseTransportBuilder::new("https://twitter-mcp.fabelis.ai".to_string()).build();
    transport.open().await?;

    let client = ClientBuilder::new(transport).use_strict().build();
    let client_clone = client.clone();
    tokio::spawn(async move { client_clone.start().await });

    let init_res = client
        .initialize(Implementation {
            name: "fabelis-twitter".to_string(),
            version: "0.1.0".to_string(),
        })
        .await?;
    println!("Initialized: {:?}", init_res);

    let response = client.list_tools(None, None).await?;
    println!("Tools: {:?}", response);

    Ok(())
}

#[tokio::test]
async fn test_sse_client_call_tool() -> Result<(), anyhow::Error> {
    dotenv().ok();

    let transport =
        ClientSseTransportBuilder::new("https://discord-mcp.fabelis.ai".to_string()).build();
    transport.open().await?;

    let client = ClientBuilder::new(transport)
        .use_strict()
        .with_secure_value(
            "discord_token",
            SecureValue::Env("DISCORD_TOKEN".to_string()),
        )
        .with_secure_value(
            "discord_channel_id",
            SecureValue::Env("DISCORD_CHANNEL_ID".to_string()),
        )
        .build();
    let client_clone = client.clone();
    tokio::spawn(async move { client_clone.start().await });

    let init_res = client
        .initialize(Implementation {
            name: "fabelis-twitter".to_string(),
            version: "0.1.0".to_string(),
        })
        .await?;
    println!("Initialized: {:?}", init_res);

    let response = client
        .call_tool(
            "PostMessage",
            Some(json!({
                "content": "Hello, world!",
                "discord_token": "",
                "discord_channel_id": ""
            })),
        )
        .await?;

    assert!(response.is_error == Some(false));

    println!("Response: {:?}", response);

    Ok(())
}
