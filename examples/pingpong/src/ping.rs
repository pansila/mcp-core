use mcp_core::{
    tool_text_response,
    tools::ToolHandlerFn,
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

    pub async fn call() -> ToolHandlerFn {
        move |_: CallToolRequest| Box::pin(async move { tool_text_response!("pong".to_string()) })
    }
}
