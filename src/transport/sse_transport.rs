use super::{Message, Transport};
use crate::sse::middleware::{AuthConfig, Claims};
use anyhow::Result;
use async_trait::async_trait;
use futures::TryStreamExt;
use jsonwebtoken::{encode, EncodingKey, Header};
use reqwest_eventsource::{Event, EventSource};

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::debug;

const CHUNK_SIZE: usize = 16 * 1024; // 16KB chunks

#[derive(Clone)]
pub struct ServerSseTransport {
    // For receiving messages from HTTP POST requests
    message_rx: Arc<Mutex<mpsc::Receiver<Message>>>,
    message_tx: mpsc::Sender<Message>,
    // For sending messages to SSE clients
    sse_tx: broadcast::Sender<Message>,
}

impl ServerSseTransport {
    pub fn new(sse_tx: broadcast::Sender<Message>) -> Self {
        let (message_tx, message_rx) = mpsc::channel(100);
        Self {
            message_rx: Arc::new(Mutex::new(message_rx)),
            message_tx,
            sse_tx,
        }
    }

    pub async fn send_message(&self, message: Message) -> Result<()> {
        self.message_tx.send(message).await?;
        Ok(())
    }

    // Helper function to chunk message into SSE format
    fn format_sse_message(message: &Message) -> Result<String> {
        let json = serde_json::to_string(message)?;
        let mut result = String::new();

        // Add event type
        result.push_str("event: message\n");

        // If small enough, send as single chunk
        if json.len() <= CHUNK_SIZE {
            result.push_str(&format!("data: {}\n\n", json));
            return Ok(result);
        }

        // For larger messages, split at proper boundaries (commas or spaces)
        let mut start = 0;
        while start < json.len() {
            let mut end = (start + CHUNK_SIZE).min(json.len());

            // If we're not at the end, find a good split point
            if end < json.len() {
                // Look back for a comma or space to split at
                while end > start && !json[end..].starts_with([',', ' ']) {
                    end -= 1;
                }
                // If we couldn't find a good split point, just use the max size
                if end == start {
                    end = (start + CHUNK_SIZE).min(json.len());
                }
            }

            result.push_str(&format!("data: {}\n", &json[start..end]));
            start = end;
        }

        result.push('\n');
        Ok(result)
    }
}

#[async_trait]
impl Transport for ServerSseTransport {
    async fn receive(&self) -> Result<Option<Message>> {
        let mut rx = self.message_rx.lock().await;
        match rx.recv().await {
            Some(message) => {
                debug!("Received message from POST request: {:?}", message);
                Ok(Some(message))
            }
            None => Ok(None),
        }
    }

    fn send(
        &self,
        message: &Message,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + Sync + '_>> {
        let message = message.clone();
        let sse_tx = self.sse_tx.clone();
        Box::pin(async move {
            let formatted = Self::format_sse_message(&message)?;
            debug!("Sending chunked SSE message: {}", formatted);
            sse_tx.send(message)?;
            Ok(())
        })
    }

    async fn open(&self) -> Result<()> {
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub enum SseEvent {
    Message(Message),
    Session(String),
}

/// Client-side SSE transport that sends messages via HTTP POST
/// and receives responses via SSE
#[derive(Clone)]
pub struct ClientSseTransport {
    tx: mpsc::Sender<Message>,
    rx: Arc<Mutex<mpsc::Receiver<Message>>>,
    server_url: String,
    client: reqwest::Client,
    auth_config: Option<AuthConfig>,
    bearer_token: Option<String>,
    session_endpoint: Arc<Mutex<Option<String>>>,
    headers: HashMap<String, String>,
}

impl ClientSseTransport {
    pub fn builder(url: String) -> ClientSseTransportBuilder {
        ClientSseTransportBuilder::new(url)
    }

    fn generate_token(&self) -> Result<Option<String>> {
        if let Some(bearer_token) = &self.bearer_token {
            return Ok(Some(bearer_token.clone()));
        }

        let auth_config = match self.auth_config.as_ref() {
            Some(config) => config,
            None => return Ok(None),
        };

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize;
        let claims = Claims {
            iat: now,
            exp: now + 3600, // Token expires in 1 hour
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(auth_config.jwt_secret.as_bytes()),
        )
        .map_err(Into::into)
        .map(Some)
    }
}

#[derive(Default)]
pub struct ClientSseTransportBuilder {
    server_url: String,
    auth_config: Option<AuthConfig>,
    bearer_token: Option<String>,
    headers: HashMap<String, String>,
}

impl ClientSseTransportBuilder {
    pub fn new(server_url: String) -> Self {
        Self {
            server_url,
            auth_config: None,
            bearer_token: None,
            headers: HashMap::new(),
        }
    }

    pub fn with_auth(mut self, jwt_secret: String) -> Self {
        self.auth_config = Some(AuthConfig { jwt_secret });
        self
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
        let (tx, rx) = mpsc::channel(100);
        ClientSseTransport {
            tx,
            rx: Arc::new(Mutex::new(rx)),
            server_url: self.server_url,
            client: reqwest::Client::new(),
            auth_config: self.auth_config,
            bearer_token: self.bearer_token,
            session_endpoint: Arc::new(Mutex::new(None)),
            headers: self.headers,
        }
    }
}

#[async_trait]
impl Transport for ClientSseTransport {
    async fn receive(&self) -> Result<Option<Message>> {
        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Some(message) => {
                debug!("Received SSE message: {:?}", message);
                Ok(Some(message))
            }
            None => Ok(None),
        }
    }

    fn send(
        &self,
        message: &Message,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + Sync + '_>> {
        let message = message.clone();
        Box::pin(async move {
            let session_url = {
                let url = self.session_endpoint.lock().await;
                format!(
                    "{}{}",
                    self.server_url,
                    url.as_ref()
                        .ok_or_else(|| anyhow::anyhow!("No session URL available"))?
                        .clone()
                )
            };

            let mut request = self.client.post(session_url).json(&message);

            if let Some(token) = self.generate_token()? {
                request = request.header("Authorization", format!("Bearer {}", token))
            };

            let response = request.send().await?;

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await?;
                return Err(anyhow::anyhow!(
                    "Failed to send message, status: {status}, body: {text}",
                ));
            }

            Ok(())
        })
    }

    async fn open(&self) -> Result<()> {
        let tx = self.tx.clone();
        let server_url = self.server_url.clone();
        let auth_config = self.auth_config.clone();
        let bearer_token = self.bearer_token.clone();
        let session_endpoint = self.session_endpoint.clone();
        let headers = self.headers.clone();

        let handle = tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut request = client.get(&format!("{}/sse", server_url));

            // Add custom headers
            for (key, value) in &headers {
                request = request.header(key, value);
            }

            // Add auth header if configured
            if let Some(bearer_token) = bearer_token {
                request = request.header("Authorization", format!("Bearer {}", bearer_token));
            } else if let Some(auth_config) = auth_config {
                let claims = Claims {
                    iat: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize,
                    exp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize + 3600,
                };

                let token = encode(
                    &Header::default(),
                    &claims,
                    &EncodingKey::from_secret(auth_config.jwt_secret.as_bytes()),
                )?;

                request = request.header("Authorization", format!("Bearer {}", token));
            }

            let mut sse = match EventSource::new(request) {
                Ok(es) => es,
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to create EventSource: {}", e));
                }
            };

            while let Some(event_result) = sse.try_next().await.transpose() {
                match event_result {
                    Ok(event) => match event {
                        Event::Message(m) => match &m.event[..] {
                            "endpoint" => {
                                let endpoint = m
                                    .data
                                    .trim_start_matches("http://")
                                    .trim_start_matches("https://")
                                    .split_once('/')
                                    .map(|(_, path)| format!("/{}", path))
                                    .unwrap_or(m.data);
                                tracing::info!("Received session endpoint: {}", endpoint);
                                *session_endpoint.lock().await = Some(endpoint);
                            }
                            _ => {
                                tx.send(serde_json::from_str(&m.data)?).await?;
                            }
                        },
                        _ => continue,
                    },
                    Err(e) => {
                        tracing::error!("Failed to parse SSE message: {:?}", e);
                        continue;
                    }
                }
            }

            Ok::<_, anyhow::Error>(())
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

        handle.abort();
        Err(anyhow::anyhow!("Timeout waiting for initial SSE message"))
    }

    async fn close(&self) -> Result<()> {
        // Clear the session URL to prevent further message sending
        *self.session_endpoint.lock().await = None;

        // Close the message channel by dropping the sender
        drop(self.tx.clone());

        // Wait for any pending messages to be processed
        let mut rx = self.rx.lock().await;
        while rx.try_recv().is_ok() {}

        Ok(())
    }
}
