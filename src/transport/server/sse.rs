use crate::{
    protocol::{Protocol, RequestOptions},
    transport::{
        JsonRpcError, JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
        Message, RequestId, Transport,
    },
    types::ErrorCode,
};
use actix_web::{
    middleware::Logger,
    web::{self, Query},
    App, HttpResponse, HttpServer,
};
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use std::pin::Pin;
use std::sync::Arc;
use std::{collections::HashMap, future::Future};
use tokio::{
    sync::{mpsc, Mutex},
    time::timeout,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct ServerSseTransport {
    protocol: Protocol,
    sessions: Arc<Mutex<HashMap<String, ServerSseTransportSession>>>,
    host: String,
    port: u16,
}

impl ServerSseTransport {
    pub fn new(host: String, port: u16, protocol: Protocol) -> Self {
        Self {
            protocol,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            host,
            port,
        }
    }

    async fn create_session(&self, session_id: String) {
        let (tx, rx) = mpsc::channel::<JsonRpcMessage>(100);
        let session = ServerSseTransportSession {
            protocol: self.protocol.clone(),
            tx,
            rx: Arc::new(Mutex::new(rx)),
        };
        self.sessions.lock().await.insert(session_id, session);
    }

    async fn get_session(&self, session_id: &str) -> Option<ServerSseTransportSession> {
        let sessions = self.sessions.lock().await;
        sessions.get(session_id).cloned()
    }
}

#[async_trait()]
impl Transport for ServerSseTransport {
    async fn open(&self) -> Result<()> {
        let transport = self.clone();
        let server = HttpServer::new(move || {
            App::new()
                .wrap(Logger::default())
                .app_data(web::Data::new(transport.clone()))
                .route("/sse", web::get().to(sse_handler))
                .route("/message", web::post().to(message_handler))
        })
        .bind((self.host.clone(), self.port))?
        .run();

        server
            .await
            .map_err(|e| anyhow::anyhow!("Server error: {:?}", e))
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    async fn poll_message(&self) -> Result<Option<Message>> {
        Ok(None)
    }

    fn request(
        &self,
        _method: &str,
        _params: Option<serde_json::Value>,
        _options: RequestOptions,
    ) -> Pin<Box<dyn Future<Output = Result<JsonRpcResponse>> + Send>> {
        Box::pin(async move { Ok(JsonRpcResponse::default()) })
    }

    async fn send_notification(
        &self,
        _method: &str,
        _params: Option<serde_json::Value>,
    ) -> Result<()> {
        Ok(())
    }

    async fn send_response(
        &self,
        _id: RequestId,
        _result: Option<serde_json::Value>,
        _error: Option<JsonRpcError>,
    ) -> Result<()> {
        Ok(())
    }
}

pub async fn sse_handler(
    req: actix_web::HttpRequest,
    transport: web::Data<ServerSseTransport>,
) -> HttpResponse {
    let client_ip = req
        .peer_addr()
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    tracing::info!("New SSE connection request from {}", client_ip);

    // Create new session
    let session_id = Uuid::new_v4().to_string();

    transport.create_session(session_id.clone()).await;

    tracing::info!(
        "SSE connection established for {} with session_id {}",
        client_ip,
        session_id
    );

    // Create initial endpoint info event
    let endpoint_info = format!(
        "event: endpoint\ndata: /message?sessionId={}\n\n",
        session_id
    );

    let stream = futures::stream::once(async move {
        Ok::<_, std::convert::Infallible>(web::Bytes::from(endpoint_info))
    })
    .chain(futures::stream::unfold(
        (transport.clone(), session_id.clone(), client_ip.clone()),
        move |state| async move {
            let (transport, session_id, client_ip) = state;
            let session = transport.get_session(&session_id).await;

            if let Some(session) = session {
                match session.poll_message().await {
                    Ok(Some(msg)) => {
                        tracing::debug!("Sending SSE message to Session {}: {:?}", session_id, msg);
                        let json = serde_json::to_string(&msg).unwrap();
                        let sse_data = format!("data: {}\n\n", json);
                        let response =
                            Ok::<_, std::convert::Infallible>(web::Bytes::from(sse_data));
                        Some((response, (transport, session_id, client_ip)))
                    }
                    Ok(None) => None,
                    Err(e) => {
                        tracing::error!("Error polling message for Session {}: {:?}", client_ip, e);
                        None
                    }
                }
            } else {
                tracing::warn!("Session {} not found, closing stream", session_id);
                None
            }
        },
    ));

    HttpResponse::Ok()
        .append_header(("X-Session-Id", session_id))
        .content_type("text/event-stream")
        .streaming(stream)
}

#[derive(Deserialize)]
pub struct MessageQuery {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

async fn message_handler(
    query: Query<MessageQuery>,
    message: web::Json<Message>,
    transport: web::Data<ServerSseTransport>,
) -> HttpResponse {
    if let Some(session_id) = &query.session_id {
        let sessions = transport.sessions.lock().await;
        if let Some(transport) = sessions.get(session_id) {
            match message.into_inner() {
                JsonRpcMessage::Request(request) => {
                    let response = transport.protocol.handle_request(request).await;
                    match transport
                        .send_response(response.id, response.result, response.error)
                        .await
                    {
                        Ok(_) => {
                            tracing::debug!("Successfully sent message to session {}", session_id);
                            HttpResponse::Accepted().finish()
                        }
                        Err(e) => {
                            tracing::error!(
                                "Failed to send message to session {}: {:?}",
                                session_id,
                                e
                            );
                            HttpResponse::InternalServerError().finish()
                        }
                    }
                }
                JsonRpcMessage::Response(response) => {
                    transport.protocol.handle_response(response).await;
                    HttpResponse::Accepted().finish()
                }
                JsonRpcMessage::Notification(notification) => {
                    transport.protocol.handle_notification(notification).await;
                    HttpResponse::Accepted().finish()
                }
            }
        } else {
            HttpResponse::NotFound().body(format!("Session {} not found", session_id))
        }
    } else {
        HttpResponse::BadRequest().body("Session ID not specified")
    }
}
#[derive(Clone)]
pub struct ServerSseTransportSession {
    protocol: Protocol,
    rx: Arc<Mutex<mpsc::Receiver<Message>>>,
    tx: mpsc::Sender<Message>,
}

#[async_trait()]
impl Transport for ServerSseTransportSession {
    async fn open(&self) -> Result<()> {
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    async fn poll_message(&self) -> Result<Option<Message>> {
        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Some(message) => {
                tracing::debug!("Received message from SSE: {:?}", message);
                Ok(Some(message))
            }
            None => Ok(None),
        }
    }

    fn request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
        options: RequestOptions,
    ) -> Pin<Box<dyn Future<Output = Result<JsonRpcResponse>> + Send>> {
        let protocol = self.protocol.clone();
        let tx = self.tx.clone();

        let method = method.to_owned();
        let params = params.clone();

        Box::pin(async move {
            let (id, rx) = protocol.create_request().await;
            let message = JsonRpcMessage::Request(JsonRpcRequest {
                id,
                method: method.clone(),
                jsonrpc: Default::default(),
                params,
            });

            if let Err(e) = tx.send(message).await {
                return Ok(JsonRpcResponse {
                    id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: ErrorCode::InternalError as i32,
                        message: format!("Failed to send request: {}", e),
                        data: None,
                    }),
                    ..Default::default()
                });
            }

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
        let message = JsonRpcMessage::Notification(JsonRpcNotification {
            method: method.to_owned(),
            params,
            jsonrpc: Default::default(),
        });
        self.tx
            .send(message)
            .await
            .map_err(|e| anyhow::anyhow!("Send notification error: {:?}", e))
    }

    async fn send_response(
        &self,
        id: RequestId,
        result: Option<serde_json::Value>,
        error: Option<JsonRpcError>,
    ) -> Result<()> {
        let message = JsonRpcMessage::Response(JsonRpcResponse {
            id,
            result,
            error,
            jsonrpc: Default::default(),
        });
        self.tx
            .send(message)
            .await
            .map_err(|e| anyhow::anyhow!("Send response error: {:?}", e))
    }
}
