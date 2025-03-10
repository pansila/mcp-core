pub mod client;
pub mod protocol;
pub mod server;
pub mod tools;
pub mod transport;
pub mod types;

#[macro_export]
macro_rules! tool_error_response {
    ($e:expr) => {{
        let error_message = $e.to_string();
        CallToolResponse {
            content: vec![ToolResponseContent::Text {
                text: error_message,
            }],
            is_error: Some(true),
            meta: None,
        }
    }};
}

#[macro_export]
macro_rules! tool_text_response {
    ($e:expr) => {{
        CallToolResponse {
            content: vec![ToolResponseContent::Text { text: $e }],
            is_error: None,
            meta: None,
        }
    }};
}

#[macro_export]
macro_rules! tool_text_content {
    ($e:expr) => {{
        ToolResponseContent::Text { text: $e }
    }};
}

#[macro_export]
macro_rules! tool_image_content {
    ($data:expr, $mime_type:expr) => {{
        ToolResponseContent::Image {
            data: $data,
            mime_type: $mime_type,
        }
    }};
}

#[macro_export]
macro_rules! tool_resource_content {
    ($uri:expr, $mime_type:expr) => {{
        ToolResponseContent::Resource {
            resource: mcp_core::types::ResourceContents {
                uri: $uri,
                mime_type: Some($mime_type),
            },
        }
    }};
    ($uri:expr) => {{
        ToolResponseContent::Resource {
            resource: ResourceContents {
                uri: $uri,
                mime_type: None,
            },
        }
    }};
}
