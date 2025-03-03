pub mod client;
pub mod protocol;
pub mod server;
pub mod sse;
pub mod tools;
pub use sse::http_server::run_http_server;
pub mod transport;
pub mod types;

#[macro_export]
macro_rules! tool_response_error {
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
