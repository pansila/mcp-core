use std::{future::Future, pin::Pin};

use mcp_core::{
    tool_text_response,
    types::{CallToolRequest, CallToolResponse, Tool, ToolResponseContent},
};
pub struct PingTool;

impl PingTool {
    pub fn tool() -> Tool {
        Tool {
            name: "ping".to_string(),
            description: Some("Send a ping to get a pong response".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    pub async fn call(
    ) -> impl Fn(CallToolRequest) -> Pin<Box<dyn Future<Output = CallToolResponse> + Send + Sync>>
    {
        move |_: CallToolRequest| Box::pin(async move { tool_text_response!("pong".to_string()) })
    }
}
