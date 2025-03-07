use std::{future::Future, pin::Pin};

use anyhow::Result;
use mcp_core::{
    tool_error_response, tool_text_response,
    types::{CallToolRequest, CallToolResponse, Tool, ToolResponseContent},
};

use crate::tools::common::{get_path, search_directory};

pub struct SearchFilesTool;

impl SearchFilesTool {
    pub fn tool() -> Tool {
        Tool {
            name: "search_files".to_string(),
            description: Some(
                "Recursively search for files and directories matching a pattern. \
                Searches through all subdirectories from the starting path. The search \
                is case-insensitive and matches partial names. Returns full paths to all \
                matching items. Great for finding files when you don't know their exact location. \
                Only searches within allowed directories."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string"
                    },
                    "pattern": {
                        "type": "string"
                    }
                },
                "required": ["path", "pattern"]
            }),
        }
    }

    pub async fn call(
    ) -> impl Fn(CallToolRequest) -> Pin<Box<dyn Future<Output = CallToolResponse> + Send + Sync>>
    {
        move |req: CallToolRequest| {
            Box::pin(async move {
                let args = req.arguments.unwrap_or_default();
                match search_files(&args) {
                    Ok(text) => tool_text_response!(text),
                    Err(e) => tool_error_response!(format!("Error: {}", e)),
                }
            })
        }
    }
}

fn search_files(args: &std::collections::HashMap<String, serde_json::Value>) -> Result<String> {
    let path = get_path(args)?;
    let pattern = args["pattern"]
        .as_str()
        .ok_or(anyhow::anyhow!("Missing pattern"))?;
    let mut matches = Vec::new();
    search_directory(&path, pattern, &mut matches)?;
    Ok(matches.join("\n"))
}
