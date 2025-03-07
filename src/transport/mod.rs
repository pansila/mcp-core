//! Transport layer for the MCP protocol
//! handles the serialization and deserialization of message
//! handles send and receive of messages
//! defines transport layer types

use std::{future::Future, pin::Pin};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

mod client;
pub use client::*;

mod server;
pub use server::*;

use crate::protocol::RequestOptions;

/// only JsonRpcMessage is supported for now
/// https://spec.modelcontextprotocol.io/specification/basic/messages/
pub type Message = JsonRpcMessage;

#[async_trait()]
pub trait Transport: Send + Sync + 'static {
    /// open the transport
    async fn open(&self) -> Result<()>;

    /// Close the transport
    async fn close(&self) -> Result<()>;

    async fn poll_message(&self) -> Result<Option<Message>>;

    fn request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
        options: RequestOptions,
    ) -> Pin<Box<dyn Future<Output = Result<JsonRpcResponse>> + Send + Sync>>;

    async fn send_notification(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<()>;

    async fn send_response(
        &self,
        id: RequestId,
        result: Option<serde_json::Value>,
        error: Option<JsonRpcError>,
    ) -> Result<()>;
}

/// Request ID type
pub type RequestId = u64;
/// JSON RPC version type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct JsonRpcVersion(String);

impl Default for JsonRpcVersion {
    fn default() -> Self {
        JsonRpcVersion("2.0".to_owned())
    }
}

impl JsonRpcVersion {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Response(JsonRpcResponse),
    Request(JsonRpcRequest),
    Notification(JsonRpcNotification),
}

// json rpc types
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JsonRpcRequest {
    pub id: RequestId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    pub jsonrpc: JsonRpcVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct JsonRpcNotification {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    pub jsonrpc: JsonRpcVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct JsonRpcResponse {
    /// The request ID this response corresponds to
    pub id: RequestId,
    /// The result of the request, if successful
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// The error, if the request failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    /// The JSON-RPC version
    pub jsonrpc: JsonRpcVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct JsonRpcError {
    /// Error code
    pub code: i32,
    /// Error message
    pub message: String,
    /// Optional additional error data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}
