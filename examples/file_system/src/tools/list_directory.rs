use std::{future::Future, pin::Pin};

use anyhow::Result;
use mcp_core::{
    tool_error_response, tool_text_response,
    types::{CallToolRequest, CallToolResponse, Tool, ToolResponseContent},
};

use crate::tools::common::get_path;

pub struct ListDirectoryTool;

impl ListDirectoryTool {
    pub fn tool() -> Tool {
        Tool {
            name: "list_directory".to_string(),
            description: Some(
                "Get a detailed listing of all files and directories in a specified path. \
                Results clearly distinguish between files and directories with [FILE] and [DIR] \
                prefixes. This tool is essential for understanding directory structure and \
                finding specific files within a directory. Only works within allowed directories."
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
                match list_directory(&args) {
                    Ok(text) => tool_text_response!(text),
                    Err(e) => tool_error_response!(format!("Error: {}", e)),
                }
            })
        }
    }
}

fn list_directory(args: &std::collections::HashMap<String, serde_json::Value>) -> Result<String> {
    let path = get_path(args)?;
    let entries = std::fs::read_dir(path)?;
    let mut text = String::new();
    for entry in entries {
        let entry = entry?;
        let prefix = if entry.file_type()?.is_dir() {
            "[DIR]"
        } else {
            "[FILE]"
        };
        text.push_str(&format!(
            "{prefix} {}\n",
            entry.file_name().to_string_lossy()
        ));
    }
    Ok(text)
}
