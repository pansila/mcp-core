pub mod client;
pub mod protocol;
pub mod server;
#[cfg(feature = "http")]
pub mod sse;
#[cfg(feature = "http")]
pub use sse::http_server::run_http_server;
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
