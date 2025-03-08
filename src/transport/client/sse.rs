use crate::protocol::{Protocol, ProtocolBuilder, RequestOptions};
use crate::transport::{
    JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, Message, RequestId,
    Transport,
};
use crate::types::ErrorCode;
use anyhow::Result;
use async_trait::async_trait;
use futures::TryStreamExt;
use reqwest_eventsource::{Event, EventSource};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::debug;

/// ClientSseTransport communicates with a server via SSE for receiving and HTTP for sending.
#[derive(Clone)]
pub struct ClientSseTransport {
    protocol: Protocol,
    server_url: String,
    client: reqwest::Client,
    bearer_token: Option<String>,
    session_endpoint: Arc<Mutex<Option<String>>>,
    headers: HashMap<String, String>,
    event_source: Arc<Mutex<Option<EventSource>>>,
}

pub struct ClientSseTransportBuilder {
    server_url: String,
    bearer_token: Option<String>,
    headers: HashMap<String, String>,
    protocol_builder: ProtocolBuilder,
}

impl ClientSseTransportBuilder {
    pub fn new(server_url: String) -> Self {
        Self {
            server_url,
            bearer_token: None,
            headers: HashMap::new(),
            protocol_builder: ProtocolBuilder::new(),
        }
    }

    pub fn with_bearer_token(mut self, token: String) -> Self {
        self.bearer_token = Some(token);
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    pub fn build(self) -> ClientSseTransport {
        ClientSseTransport {
            protocol: self.protocol_builder.build(),
            server_url: self.server_url,
            client: reqwest::Client::new(),
            bearer_token: self.bearer_token,
            session_endpoint: Arc::new(Mutex::new(None)),
            headers: self.headers,
            event_source: Arc::new(Mutex::new(None)),
        }
    }
}

impl ClientSseTransport {
    pub fn builder(url: String) -> ClientSseTransportBuilder {
        ClientSseTransportBuilder::new(url)
    }
}

#[async_trait()]
impl Transport for ClientSseTransport {
    async fn open(&self) -> Result<()> {
        debug!("ClientSseTransport: Opening transport");

        let mut request = self.client.get(self.server_url.clone());

        // Add custom headers
        for (key, value) in &self.headers {
            request = request.header(key, value);
        }

        // Add auth header if configured
        if let Some(bearer_token) = &self.bearer_token {
            request = request.header("Authorization", format!("Bearer {}", bearer_token));
        }

        let event_source = EventSource::new(request)?;

        {
            let mut es_lock = self.event_source.lock().await;
            *es_lock = Some(event_source);
        }

        // Spawn a background task to continuously poll messages
        let transport_clone = self.clone();
        tokio::task::spawn(async move {
            loop {
                match transport_clone.poll_message().await {
                    Ok(Some(message)) => match message {
                        Message::Request(request) => {
                            let response = transport_clone.protocol.handle_request(request).await;
                            let _ = transport_clone
                                .send_response(response.id, response.result, response.error)
                                .await;
                        }
                        Message::Notification(notification) => {
                            let _ = transport_clone
                                .protocol
                                .handle_notification(notification)
                                .await;
                        }
                        Message::Response(response) => {
                            transport_clone.protocol.handle_response(response).await;
                        }
                    },
                    Ok(None) => continue, // No message or control message, continue polling
                    Err(e) => {
                        debug!("ClientSseTransport: Error polling message: {:?}", e);
                        // Maybe add some backoff or retry logic here
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });

        // Wait for the session URL to be set
        let mut attempts = 0;
        while attempts < 10 {
            if self.session_endpoint.lock().await.is_some() {
                return Ok(());
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            attempts += 1;
        }

        Err(anyhow::anyhow!("Timeout waiting for initial SSE message"))
    }

    async fn close(&self) -> Result<()> {
        debug!("ClientSseTransport: Closing transport");
        // Close the event source
        *self.event_source.lock().await = None;

        // Clear the session URL
        *self.session_endpoint.lock().await = None;

        Ok(())
    }

    async fn poll_message(&self) -> Result<Option<Message>> {
        let mut event_source_guard = self.event_source.lock().await;
        let event_source = event_source_guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Transport not opened"))?;

        match event_source.try_next().await {
            Ok(Some(event)) => match event {
                Event::Message(m) => {
                    if &m.event[..] == "endpoint" {
                        let endpoint = m
                            .data
                            .trim_start_matches("http://")
                            .trim_start_matches("https://")
                            .split_once('/')
                            .map(|(_, path)| format!("/{}", path))
                            .unwrap_or(m.data);
                        debug!("Received session endpoint: {}", endpoint);
                        *self.session_endpoint.lock().await = Some(endpoint);
                        return Ok(None); // This is a control message, not a JSON-RPC message
                    } else {
                        debug!("Received SSE message: {}", m.data);
                        let message: Message = serde_json::from_str(&m.data)?;
                        return Ok(Some(message));
                    }
                }
                _ => return Ok(None),
            },
            Ok(None) => return Ok(None), // Stream ended
            Err(e) => {
                debug!("Error receiving SSE message: {:?}", e);
                return Err(anyhow::anyhow!("Failed to parse SSE message: {:?}", e));
            }
        }
    }

    fn request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
        options: RequestOptions,
    ) -> Pin<Box<dyn Future<Output = Result<JsonRpcResponse>> + Send + Sync>> {
        let protocol = self.protocol.clone();
        let client = self.client.clone();
        let server_url = self.server_url.clone();
        let session_endpoint = self.session_endpoint.clone();
        let bearer_token = self.bearer_token.clone();
        let method = method.to_owned();

        Box::pin(async move {
            let (id, rx) = protocol.create_request().await;
            let request = JsonRpcRequest {
                id,
                method,
                jsonrpc: Default::default(),
                params,
            };

            // Get the session URL
            let session_url = {
                let url = session_endpoint.lock().await;
                url.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("No session URL available"))?
                    .clone()
            };

            let base_url = if let Some(idx) = server_url.find("://") {
                let domain_start = idx + 3;
                let domain_end = server_url[domain_start..]
                    .find('/')
                    .map(|i| domain_start + i)
                    .unwrap_or(server_url.len());
                &server_url[..domain_end]
            } else {
                let domain_end = server_url.find('/').unwrap_or(server_url.len());
                &server_url[..domain_end]
            }
            .to_string();

            debug!("ClientSseTransport: Base URL: {}", base_url);

            let full_url = format!("{}{}", base_url, session_url);
            debug!(
                "ClientSseTransport: Sending request to {}: {:?}",
                full_url, request
            );

            let mut req_builder = client.post(&full_url).json(&request);

            if let Some(token) = bearer_token {
                req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
            }

            let response = req_builder.send().await?;

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await?;
                return Err(anyhow::anyhow!(
                    "Failed to send request, status: {status}, body: {text}"
                ));
            }

            debug!("ClientSseTransport: Request sent successfully");

            // Wait for the response with a timeout
            let result = timeout(options.timeout, rx).await;
            match result {
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
                Err(_) => {
                    protocol.cancel_response(id).await;
                    Ok(JsonRpcResponse {
                        id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: ErrorCode::RequestTimeout as i32,
                            message: "Request timed out".to_string(),
                            data: None,
                        }),
                        ..Default::default()
                    })
                }
            }
        })
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

        // Get the session URL
        let session_url = {
            let url = self.session_endpoint.lock().await;
            url.as_ref()
                .ok_or_else(|| anyhow::anyhow!("No session URL available"))?
                .clone()
        };

        let server_url = self.server_url.clone();
        let base_url = if let Some(idx) = server_url.find("://") {
            let domain_start = idx + 3;
            let domain_end = server_url[domain_start..]
                .find('/')
                .map(|i| domain_start + i)
                .unwrap_or(server_url.len());
            &server_url[..domain_end]
        } else {
            let domain_end = server_url.find('/').unwrap_or(server_url.len());
            &server_url[..domain_end]
        }
        .to_string();

        debug!("ClientSseTransport: Base URL: {}", base_url);

        let full_url = format!("{}{}", base_url, session_url);
        debug!(
            "ClientSseTransport: Sending response to {}: {:?}",
            full_url, response
        );

        let mut req_builder = self.client.post(&full_url).json(&response);

        if let Some(token) = &self.bearer_token {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
        }

        let response = req_builder.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            return Err(anyhow::anyhow!(
                "Failed to send response, status: {status}, body: {text}"
            ));
        }

        Ok(())
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

        // Get the session URL
        let session_url = {
            let url = self.session_endpoint.lock().await;
            url.as_ref()
                .ok_or_else(|| anyhow::anyhow!("No session URL available"))?
                .clone()
        };

        let server_url = self.server_url.clone();
        let base_url = if let Some(idx) = server_url.find("://") {
            let domain_start = idx + 3;
            let domain_end = server_url[domain_start..]
                .find('/')
                .map(|i| domain_start + i)
                .unwrap_or(server_url.len());
            &server_url[..domain_end]
        } else {
            let domain_end = server_url.find('/').unwrap_or(server_url.len());
            &server_url[..domain_end]
        }
        .to_string();

        debug!("ClientSseTransport: Base URL: {}", base_url);

        let full_url = format!("{}{}", base_url, session_url);
        debug!(
            "ClientSseTransport: Sending notification to {}: {:?}",
            full_url, notification
        );

        let mut req_builder = self.client.post(&full_url).json(&notification);

        if let Some(token) = &self.bearer_token {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
        }

        let response = req_builder.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            return Err(anyhow::anyhow!(
                "Failed to send notification, status: {status}, body: {text}"
            ));
        }

        Ok(())
    }
}
