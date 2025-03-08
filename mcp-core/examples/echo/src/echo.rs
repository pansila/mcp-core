use anyhow::Result;
use mcp_core::types::ToolResponseContent;
use mcp_core_macros::tool;

#[tool(
    name = "echo",
    description = "Echo back the message you send",
    params(message = "The message to echo back")
)]
async fn echo_tool(message: String) -> Result<ToolResponseContent> {
    Ok(ToolResponseContent::Text { text: message })
}
