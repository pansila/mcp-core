use std::{future::Future, pin::Pin};

use anyhow::Result;
use mcp_core::{
    tool_error_response, tool_text_response,
    types::{CallToolRequest, CallToolResponse, Tool, ToolResponseContent},
};

use crate::tools::common::get_path;

pub struct ReadFileTool;

impl ReadFileTool {
    pub fn tool() -> Tool {
        Tool {
            name: "read_file".to_string(),
            description: Some(
                "Read the complete contents of a file from the file system. \
                Handles various text encodings and provides detailed error messages \
                if the file cannot be read. Use this tool when you need to examine \
                the contents of a single file. Only works within allowed directories."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    pub async fn call(
    ) -> impl Fn(CallToolRequest) -> Pin<Box<dyn Future<Output = CallToolResponse> + Send + Sync>> {
        move |req: CallToolRequest| {
            Box::pin(async move {
                let args = req.arguments.unwrap_or_default();
                match read_file(&args) {
                    Ok(text) => tool_text_response!(text),
                    Err(e) => tool_error_response!(format!("Error: {}", e)),
                }
            })
        }
    }
}

fn read_file(args: &std::collections::HashMap<String, serde_json::Value>) -> Result<String> {
    let path = get_path(args)?;
    let content = std::fs::read_to_string(path)?;
    Ok(content)
}
