use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::{
    error::McpError,
    protocol::types::*, // Import all JSON-RPC types from protocol
};

pub use sse::ClientTransport as SseClientTransport;
pub use sse::ServerTransport as SseServerTransport;
pub use stdio::ClientTransport as StdioClientTransport;
pub use stdio::ServerTransport as StdioServerTransport;

// Message types for the transport actor
#[derive(Debug)]
pub enum TransportCommand {
    SendMessage(JsonRpcMessage),
    Close,
}

#[derive(Debug)]
pub enum TransportEvent {
    Message(JsonRpcMessage),
    Error(McpError),
    Closed,
}

#[async_trait]
pub trait ServerTransportTrait: Send + Sync + Sized + 'static {
    async fn start(&self) -> Result<TransportChannels, McpError>;
}

#[async_trait]
pub trait ClientTransportTrait: Send + Sync + Sized + 'static {
    async fn start(&self) -> Result<TransportChannels, McpError>;
}

// Channels for communicating with the transport
#[derive(Debug, Clone)]
pub struct TransportChannels {
    /// Send commands to the transport
    pub cmd_tx: mpsc::Sender<TransportCommand>,
    /// Receive events from the transport
    pub event_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<TransportEvent>>>,
}

// Stdio Transport Implementation
pub mod stdio;

// SSE Transport Implementation
pub mod sse;
