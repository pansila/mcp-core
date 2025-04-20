use crate::protocol::{Protocol, RequestOptions};
use crate::transport::{
    JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, Message, RequestId,
    Transport,
};
use crate::types::ErrorCode;
use anyhow::Result;
use async_trait::async_trait;
use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;
use tokio::time::timeout;
use tracing::debug;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Clone)]
pub struct ServerStdioTransport {
    protocol: Protocol,
}

impl ServerStdioTransport {
    pub fn new(protocol: Protocol) -> Self {
        Self { protocol }
    }
}

#[async_trait()]
impl Transport for ServerStdioTransport {
    async fn open(&self) -> Result<()> {
        loop {
            match self.poll_message().await {
                Ok(Some(message)) => match message {
                    Message::Request(request) => {
                        let response = self.protocol.handle_request(request).await;
                        self.send_response(response.id, response.result, response.error)
                            .await?;
                    }
                    Message::Notification(notification) => {
                        self.protocol.handle_notification(notification).await;
                    }
                    Message::Response(response) => {
                        self.protocol.handle_response(response).await;
                    }
                },
                Ok(None) => {
                    break;
                }
                Err(e) => {
                    tracing::error!("Error receiving message: {:?}", e);
                }
            }
        }
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    async fn poll_message(&self) -> Result<Option<Message>> {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        if line.is_empty() {
            return Ok(None);
        }

        debug!("Received: {line}");
        let message: Message = serde_json::from_str(&line)?;
        Ok(Some(message))
    }

    fn request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
        options: RequestOptions,
    ) -> Pin<Box<dyn Future<Output = Result<JsonRpcResponse>> + Send + Sync>> {
        let protocol = self.protocol.clone();
        let method = method.to_owned();
        Box::pin(async move {
            let (id, rx) = protocol.create_request().await;
            let request = JsonRpcRequest {
                id,
                method,
                jsonrpc: Default::default(),
                params,
            };
            let serialized = serde_json::to_string(&request).unwrap_or_default();
            debug!("Sending: {serialized}");

            // Use Tokio's async stdout to perform thread-safe, nonblocking writes.
            let mut stdout = io::stdout();
            stdout.write_all(serialized.as_bytes())?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;

            let result = timeout(options.timeout, rx).await;
            match result {
                // The request future completed before the timeout.
                Ok(inner_result) => match inner_result {
                    Ok(response) => Ok(response),
                    Err(_) => {
                        protocol.cancel_response(id).await;
                        Ok(JsonRpcResponse {
                            id,
                            result: None,
                            error: Some(JsonRpcError {
                                code: ErrorCode::RequestTimeout as i32,
                                message: "Request cancelled".to_string(),
                                data: None,
                            }),
                            ..Default::default()
                        })
                    }
                },
                // The timeout expired.
                Err(_) => {
                    protocol.cancel_response(id).await;
                    Ok(JsonRpcResponse {
                        id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: ErrorCode::RequestTimeout as i32,
                            message: "Request cancelled".to_string(),
                            data: None,
                        }),
                        ..Default::default()
                    })
                }
            }
        })
    }

    async fn send_notification(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<()> {
        let notification = JsonRpcNotification {
            jsonrpc: Default::default(),
            method: method.to_owned(),
            params,
        };
        let serialized = serde_json::to_string(&notification).unwrap_or_default();
        let stdout = io::stdout();
        let mut writer = stdout.lock();
        debug!("Sending: {serialized}");
        writer.write_all(serialized.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    async fn send_response(
        &self,
        id: RequestId,
        result: Option<serde_json::Value>,
        error: Option<JsonRpcError>,
    ) -> Result<()> {
        let response = JsonRpcResponse {
            id,
            result,
            error,
            jsonrpc: Default::default(),
        };
        let serialized = serde_json::to_string(&response).unwrap_or_default();
        let stdout = io::stdout();
        let mut writer = stdout.lock();
        debug!("Sending: {serialized}");
        writer.write_all(serialized.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }
}
