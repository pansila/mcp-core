use super::{
    ClientTransportTrait, JsonRpcMessage, ServerTransportTrait, TransportChannels,
    TransportCommand, TransportEvent, TransportTrait,
};
use crate::error::McpError;
use async_trait::async_trait;
use futures::TryStreamExt;
use reqwest_eventsource::{Event, EventSource};
use serde::{Deserialize, Serialize};
use std::{
    net::IpAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{broadcast::Sender, mpsc, Mutex};
use warp::Filter;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EndpointEvent {
    endpoint: String,
}

pub struct ServerTransport {
    host: String,
    port: u16,
    public_url: Option<String>,
    buffer_size: usize,
}

impl ServerTransport {
    pub fn new_local(host: String, port: u16, buffer_size: usize) -> Self {
        Self {
            host,
            port,
            public_url: None,
            buffer_size,
        }
    }

    pub fn new_public(host: String, port: u16, public_url: String, buffer_size: usize) -> Self {
        Self {
            host,
            port,
            public_url: Some(public_url),
            buffer_size,
        }
    }

    fn get_endpoint(&self) -> String {
        match &self.public_url {
            Some(url) => url.clone(),
            None => format!("http://{}:{}", self.host, self.port),
        }
    }
}

#[async_trait]
impl TransportTrait for ServerTransport {
    async fn start(&mut self) -> Result<TransportChannels, McpError> {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(self.buffer_size);
        let (event_tx, event_rx) = mpsc::channel(self.buffer_size);
        let (broadcast_tx, _) = tokio::sync::broadcast::channel::<JsonRpcMessage>(100);
        let broadcast_tx = Arc::new(broadcast_tx);
        let client_counter = Arc::new(AtomicU64::new(0));

        let endpoint = Arc::new(self.get_endpoint());

        let sse_route = warp::path("sse")
            .and(warp::get())
            .and(warp::any().map({
                let broadcast_tx = broadcast_tx.clone();
                let endpoint = endpoint.clone();
                let client_counter = client_counter.clone();
                move || {
                    (
                        broadcast_tx.clone(),
                        endpoint.clone(),
                        client_counter.clone(),
                    )
                }
            }))
            .and_then(
                |(broadcast_tx, endpoint, client_counter): (
                    Arc<Sender<JsonRpcMessage>>,
                    Arc<String>,
                    Arc<AtomicU64>,
                )| async move {
                    let client_id = client_counter.fetch_add(1, Ordering::SeqCst);
                    let broadcast_rx = broadcast_tx.subscribe();
                    let endpoint = format!("{}/message/{}", endpoint, client_id);

                    Ok::<_, warp::Rejection>(warp::sse::reply(
                        warp::sse::keep_alive()
                            .interval(Duration::from_secs(30))
                            .stream(async_stream::stream! {
                                yield Ok::<_, warp::Error>(warp::sse::Event::default()
                                    .event("endpoint")
                                    .data(endpoint));

                                let mut broadcast_rx = broadcast_rx;
                                while let Ok(msg) = broadcast_rx.recv().await {
                                    yield Ok::<_, warp::Error>(warp::sse::Event::default()
                                        .event("message")
                                        .json_data(&msg)
                                        .unwrap());
                                }
                            }),
                    ))
                },
            );

        let message_route = warp::path!("message" / u64)
            .and(warp::post())
            .and(warp::body::json())
            .map({
                let event_tx = event_tx.clone();
                move |_client_id: u64, message: JsonRpcMessage| {
                    let event_tx = event_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = event_tx.send(TransportEvent::Message(message)).await {
                            tracing::error!("Failed to forward message: {:?}", e);
                        }
                    });
                    warp::reply()
                }
            });

        let routes = sse_route.or(message_route);

        tokio::spawn({
            let broadcast_tx = broadcast_tx.clone();
            async move {
                while let Some(cmd) = cmd_rx.recv().await {
                    match cmd {
                        TransportCommand::SendMessage(msg) => {
                            let should_skip = match &msg {
                                JsonRpcMessage::Notification(n)
                                    if n.method == "notifications/message" =>
                                {
                                    if let Some(params) = &n.params {
                                        let is_debug = params
                                            .get("level")
                                            .and_then(|l| l.as_str())
                                            .map_or(false, |l| l == "debug");

                                        let logger = params
                                            .get("logger")
                                            .and_then(|l| l.as_str())
                                            .unwrap_or("");

                                        let message = params
                                            .get("data")
                                            .and_then(|d| d.get("message"))
                                            .and_then(|m| m.as_str())
                                            .unwrap_or("");

                                        is_debug
                                            && (logger.starts_with("hyper::")
                                                || logger.starts_with("mcp_core::transport")
                                                || message.contains("Broadcasting SSE message")
                                                || message.contains("Failed to broadcast message"))
                                    } else {
                                        false
                                    }
                                }
                                _ => false,
                            };

                            if !should_skip {
                                tracing::debug!("Broadcasting SSE message: {:?}", msg);
                                if let Err(e) = broadcast_tx.clone().send(msg) {
                                    tracing::error!("Failed to broadcast message: {:?}", e);
                                }
                            }
                        }
                        TransportCommand::Close => break,
                    }
                }
            }
        });

        tokio::spawn(warp::serve(routes).run((self.host.parse::<IpAddr>().unwrap(), self.port)));

        let event_rx = Arc::new(Mutex::new(event_rx));

        Ok(TransportChannels { cmd_tx, event_rx })
    }

    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3000,
            public_url: None,
            buffer_size: 1024,
        }
    }
}

pub struct ClientTransport {
    endpoint: String,
    buffer_size: usize,
}

impl ClientTransport {
    pub fn new(endpoint: String, buffer_size: usize) -> Self {
        Self {
            endpoint,
            buffer_size,
        }
    }

    async fn wait_for_endpoint(sse: &mut EventSource) -> Option<String> {
        while let Ok(Some(event)) = sse.try_next().await {
            if let Event::Message(m) = event {
                if m.event == "endpoint" {
                    return Some(m.data);
                }
            }
        }
        None
    }
}

#[async_trait]
impl ClientTransportTrait for ClientTransport {
    async fn start(&mut self) -> Result<TransportChannels, McpError> {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(self.buffer_size);
        let (event_tx, event_rx) = mpsc::channel(self.buffer_size);

        let endpoint = self.endpoint.clone();

        tokio::spawn({
            let endpoint = endpoint.clone();
            let event_tx = event_tx.clone();
            async move {
                let client = reqwest::Client::new();
                let sse_url = format!("{}/sse", endpoint);
                tracing::debug!("Connecting to SSE endpoint: {}", sse_url);

                let rb = client.get(&sse_url);
                let mut sse = match EventSource::new(rb) {
                    Ok(es) => es,
                    Err(e) => {
                        tracing::error!("Failed to create EventSource: {:?}", e);
                        let _ = event_tx
                            .send(TransportEvent::Error(McpError::ConnectionClosed))
                            .await;
                        return;
                    }
                };

                // Wait for the endpoint event.
                let endpoint = match Self::wait_for_endpoint(&mut sse).await {
                    Some(ep) => ep,
                    None => {
                        tracing::error!("Failed to receive endpoint");
                        let _ = event_tx
                            .send(TransportEvent::Error(McpError::ConnectionClosed))
                            .await;
                        return;
                    }
                };

                println!("Connected to server using SSE transport: {}", endpoint);
                tracing::debug!("Received message endpoint: {}", endpoint);

                // Spawn a task to receive messages from the EventSource.
                let event_tx2 = event_tx.clone();
                tokio::spawn(async move {
                    while let Ok(Some(event)) = sse.try_next().await {
                        match event {
                            Event::Message(m) if m.event == "message" => {
                                match serde_json::from_str::<JsonRpcMessage>(&m.data) {
                                    Ok(msg) => {
                                        if let Err(e) =
                                            event_tx2.send(TransportEvent::Message(msg)).await
                                        {
                                            tracing::error!(
                                                "Failed to forward SSE message: {:?}",
                                                e
                                            );
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to parse SSE message: {:?}", e);
                                    }
                                }
                            }
                            _ => continue,
                        }
                    }
                });

                // Process outgoing messages.
                while let Some(cmd) = cmd_rx.recv().await {
                    match cmd {
                        TransportCommand::SendMessage(msg) => {
                            tracing::debug!("Sending message to {}: {:?}", endpoint, msg);
                            if let Err(e) = client.post(&endpoint).json(&msg).send().await {
                                tracing::error!("Failed to send message: {:?}", e);
                            }
                        }
                        TransportCommand::Close => break,
                    }
                }

                let _ = event_tx.send(TransportEvent::Closed).await;
            }
        });

        let event_rx = Arc::new(Mutex::new(event_rx));
        Ok(TransportChannels { cmd_tx, event_rx })
    }

    fn default() -> Self {
        Self {
            endpoint: "http://localhost:3030".to_string(),
            buffer_size: 1024,
        }
    }
}
