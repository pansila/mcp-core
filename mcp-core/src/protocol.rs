use super::transport::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use super::types::ErrorCode;
use anyhow::Result;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;
use std::pin::Pin;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{oneshot, Mutex};

#[derive(Clone)]
pub struct Protocol {
    request_id: Arc<AtomicU64>,
    pending_requests: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    request_handlers: Arc<Mutex<HashMap<String, Box<dyn RequestHandler>>>>,
    notification_handlers: Arc<Mutex<HashMap<String, Box<dyn NotificationHandler>>>>,
}

impl Protocol {
    pub fn builder() -> ProtocolBuilder {
        ProtocolBuilder::new()
    }

    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let handlers = self.request_handlers.lock().await;
        if let Some(handler) = handlers.get(&request.method) {
            match handler.handle(request.clone()).await {
                Ok(response) => response,
                Err(e) => JsonRpcResponse {
                    id: request.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: ErrorCode::InternalError as i32,
                        message: e.to_string(),
                        data: None,
                    }),
                    ..Default::default()
                },
            }
        } else {
            JsonRpcResponse {
                id: request.id,
                error: Some(JsonRpcError {
                    code: ErrorCode::MethodNotFound as i32,
                    message: format!("Method not found: {}", request.method),
                    data: None,
                }),
                ..Default::default()
            }
        }
    }

    pub async fn handle_notification(&self, request: JsonRpcNotification) {
        let handlers = self.notification_handlers.lock().await;
        if let Some(handler) = handlers.get(&request.method) {
            match handler.handle(request.clone()).await {
                Ok(_) => tracing::info!("Received notification: {:?}", request.method),
                Err(e) => tracing::error!("Error handling notification: {}", e),
            }
        } else {
            tracing::debug!("No handler for notification: {}", request.method);
        }
    }

    pub async fn create_request(&self) -> (u64, oneshot::Receiver<JsonRpcResponse>) {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();

        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(id, tx);
        }

        (id, rx)
    }

    pub async fn handle_response(&self, response: JsonRpcResponse) {
        if let Some(tx) = self.pending_requests.lock().await.remove(&response.id) {
            let _ = tx.send(response);
        }
    }

    pub async fn cancel_response(&self, id: u64) {
        if let Some(tx) = self.pending_requests.lock().await.remove(&id) {
            let _ = tx.send(JsonRpcResponse {
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: ErrorCode::RequestTimeout as i32,
                    message: "Request cancelled".to_string(),
                    data: None,
                }),
                ..Default::default()
            });
        }
    }
}

/// The default request timeout, in milliseconds
pub const DEFAULT_REQUEST_TIMEOUT_MSEC: u64 = 60000;
pub struct RequestOptions {
    pub timeout: Duration,
}

impl RequestOptions {
    pub fn timeout(self, timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl Default for RequestOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_millis(DEFAULT_REQUEST_TIMEOUT_MSEC),
        }
    }
}

#[derive(Clone)]
pub struct ProtocolBuilder {
    request_handlers: Arc<Mutex<HashMap<String, Box<dyn RequestHandler>>>>,
    notification_handlers: Arc<Mutex<HashMap<String, Box<dyn NotificationHandler>>>>,
}

impl ProtocolBuilder {
    pub fn new() -> Self {
        Self {
            request_handlers: Arc::new(Mutex::new(HashMap::new())),
            notification_handlers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a typed request handler
    pub fn request_handler<Req, Resp>(
        self,
        method: &str,
        handler: impl Fn(Req) -> Pin<Box<dyn std::future::Future<Output = Result<Resp>> + Send>>
            + Send
            + Sync
            + 'static,
    ) -> Self
    where
        Req: DeserializeOwned + Send + Sync + 'static,
        Resp: Serialize + Send + Sync + 'static,
    {
        let handler = TypedRequestHandler {
            handler: Box::new(handler),
            _phantom: std::marker::PhantomData,
        };

        if let Ok(mut handlers) = self.request_handlers.try_lock() {
            handlers.insert(method.to_string(), Box::new(handler));
        }
        self
    }

    pub fn has_request_handler(&self, method: &str) -> bool {
        self.request_handlers
            .try_lock()
            .map(|handlers| handlers.contains_key(method))
            .unwrap_or(false)
    }

    pub fn notification_handler<N>(
        self,
        method: &str,
        handler: impl Fn(N) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
            + Send
            + Sync
            + 'static,
    ) -> Self
    where
        N: DeserializeOwned + Send + Sync + 'static,
    {
        let handler = TypedNotificationHandler {
            handler: Box::new(handler),
            _phantom: std::marker::PhantomData,
        };

        if let Ok(mut handlers) = self.notification_handlers.try_lock() {
            handlers.insert(method.to_string(), Box::new(handler));
        }
        self
    }

    pub fn has_notification_handler(&self, method: &str) -> bool {
        self.notification_handlers
            .try_lock()
            .map(|handlers| handlers.contains_key(method))
            .unwrap_or(false)
    }

    pub fn build(self) -> Protocol {
        Protocol {
            request_id: Arc::new(AtomicU64::new(0)),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            request_handlers: self.request_handlers,
            notification_handlers: self.notification_handlers,
        }
    }
}

// Update the handler traits to be async
#[async_trait]
trait RequestHandler: Send + Sync {
    async fn handle(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse>;
}

#[async_trait]
trait NotificationHandler: Send + Sync {
    async fn handle(&self, notification: JsonRpcNotification) -> Result<()>;
}

// Update the TypedRequestHandler to use async handlers
struct TypedRequestHandler<Req, Resp>
where
    Req: DeserializeOwned + Send + Sync + 'static,
    Resp: Serialize + Send + Sync + 'static,
{
    handler: Box<
        dyn Fn(Req) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Resp>> + Send>>
            + Send
            + Sync,
    >,
    _phantom: std::marker::PhantomData<(Req, Resp)>,
}

#[async_trait]
impl<Req, Resp> RequestHandler for TypedRequestHandler<Req, Resp>
where
    Req: DeserializeOwned + Send + Sync + 'static,
    Resp: Serialize + Send + Sync + 'static,
{
    async fn handle(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let params: Req = if request.params.is_none() || request.params.as_ref().unwrap().is_null()
        {
            serde_json::from_value(json!({}))?
        } else {
            serde_json::from_value(request.params.unwrap())?
        };
        let result = (self.handler)(params).await?;
        Ok(JsonRpcResponse {
            id: request.id,
            result: Some(serde_json::to_value(result)?),
            error: None,
            ..Default::default()
        })
    }
}

struct TypedNotificationHandler<N>
where
    N: DeserializeOwned + Send + Sync + 'static,
{
    handler: Box<
        dyn Fn(N) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
            + Send
            + Sync,
    >,
    _phantom: std::marker::PhantomData<N>,
}

#[async_trait]
impl<N> NotificationHandler for TypedNotificationHandler<N>
where
    N: DeserializeOwned + Send + Sync + 'static,
{
    async fn handle(&self, notification: JsonRpcNotification) -> Result<()> {
        let params: N =
            if notification.params.is_none() || notification.params.as_ref().unwrap().is_null() {
                serde_json::from_value(serde_json::Value::Null)?
            } else {
                serde_json::from_value(notification.params.unwrap())?
            };
        (self.handler)(params).await
    }
}
