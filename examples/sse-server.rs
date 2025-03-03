use mcp_core::{
    run_http_server,
    server::Server,
    sse::http_server::Host,
    types::{CallToolRequest, CallToolResponse, ServerCapabilities, Tool, ToolResponseContent},
};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stderr)
        .init();

    run_http_server(
        Host {
            host: "127.0.0.1".to_string(),
            port: 8080,
            public_url: None,
        },
        None,
        |transport| async move {
            let mut server_builder = Server::builder(transport)
                .capabilities(ServerCapabilities {
                    tools: Some(json!({
                        "listChanged": false,
                    })),
                    ..ServerCapabilities::default()
                })
                .version("0.1.0")
                .name("Example SSE Server");

            server_builder.register_tool(
                Tool {
                    name: "test".to_string(),
                    description: Some("Test Tool".to_string()),
                    input_schema: json!({
                       "type":"object",
                       "properties":{
                          "test_data":{
                             "type": "string",
                             "description": "Test data",
                          }
                       },
                       "required":["test_data"]
                    }),
                },
                move |req: CallToolRequest| {
                    Box::pin(async move {
                        let args = req.arguments.unwrap_or_default();
                        let data = args.get("test_data");

                        if data.is_none() {
                            return CallToolResponse {
                                content: vec![ToolResponseContent::Text {
                                    text: "Missing 'test_data' argument".to_string(),
                                }],
                                is_error: Some(true),
                                meta: None,
                            };
                        };

                        CallToolResponse {
                            content: vec![ToolResponseContent::Text {
                                text: json!(data).to_string(),
                            }],
                            is_error: None,
                            meta: None,
                        }
                    })
                },
            );

            Ok(server_builder.build())
        },
    )
    .await?;

    Ok(())
}
