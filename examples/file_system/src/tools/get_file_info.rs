use std::{future::Future, pin::Pin};

use anyhow::Result;
use mcp_core::{
    tool_error_response, tool_text_response,
    types::{CallToolRequest, CallToolResponse, Tool, ToolResponseContent},
};

use crate::tools::common::get_path;

pub struct GetFileInfoTool;

impl GetFileInfoTool {
    pub fn tool() -> Tool {
        Tool {
            name: "get_file_info".to_string(),
            description: Some(
                "Retrieve detailed metadata about a file or directory. Returns comprehensive \
                information including size, creation time, last modified time, permissions, \
                and type. This tool is perfect for understanding file characteristics \
                without reading the actual content. Only works within allowed directories."
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
                match get_file_info(&args) {
                    Ok(text) => tool_text_response!(text),
                    Err(e) => tool_error_response!(format!("Error: {}", e)),
                }
            })
        }
    }
}

fn get_file_info(args: &std::collections::HashMap<String, serde_json::Value>) -> Result<String> {
    let path = get_path(args)?;
    let metadata = std::fs::metadata(path)?;
    Ok(format!("{:#?}", metadata))
}
