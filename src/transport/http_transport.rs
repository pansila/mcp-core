use std::{future::Future, pin::Pin};

use super::{
    ClientSseTransport, ClientWsTransport, Message, ServerSseTransport, ServerWsTransport,
    Transport,
};
use anyhow::Result;
pub enum ServerHttpTransport {
    Sse(ServerSseTransport),
    Ws(ServerWsTransport),
}
pub enum ClientHttpTransport {
    Sse(ClientSseTransport),
    Ws(ClientWsTransport),
}

impl Clone for ServerHttpTransport {
    fn clone(&self) -> Self {
        match self {
            ServerHttpTransport::Sse(sse) => ServerHttpTransport::Sse(sse.clone()),
            ServerHttpTransport::Ws(ws) => ServerHttpTransport::Ws(ws.clone()),
        }
    }
}

#[async_trait::async_trait]
impl Transport for ServerHttpTransport {
    fn send(
        &self,
        message: &Message,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + Sync + '_>> {
        let transport = self.clone();
        let message = message.clone();
        Box::pin(async move {
            match transport {
                ServerHttpTransport::Sse(sse) => sse.send(&message).await,
                ServerHttpTransport::Ws(ws) => ws.send(&message).await,
            }
        })
    }

    async fn receive(&self) -> Result<Option<Message>> {
        match self {
            ServerHttpTransport::Sse(sse) => sse.receive().await,
            ServerHttpTransport::Ws(ws) => ws.receive().await,
        }
    }

    async fn open(&self) -> Result<()> {
        match self {
            ServerHttpTransport::Sse(sse) => sse.open().await,
            ServerHttpTransport::Ws(ws) => ws.open().await,
        }
    }

    async fn close(&self) -> Result<()> {
        match self {
            ServerHttpTransport::Sse(sse) => sse.close().await,
            ServerHttpTransport::Ws(ws) => ws.close().await,
        }
    }
}

impl Clone for ClientHttpTransport {
    fn clone(&self) -> Self {
        match self {
            ClientHttpTransport::Sse(sse) => ClientHttpTransport::Sse(sse.clone()),
            ClientHttpTransport::Ws(ws) => ClientHttpTransport::Ws(ws.clone()),
        }
    }
}

#[async_trait::async_trait]
impl Transport for ClientHttpTransport {
    fn send(
        &self,
        message: &Message,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + Sync + '_>> {
        let transport = self.clone();
        let message = message.clone();
        Box::pin(async move {
            match transport {
                ClientHttpTransport::Sse(sse) => sse.send(&message).await,
                ClientHttpTransport::Ws(ws) => ws.send(&message).await,
            }
        })
    }

    async fn receive(&self) -> Result<Option<Message>> {
        match self {
            ClientHttpTransport::Sse(sse) => sse.receive().await,
            ClientHttpTransport::Ws(ws) => ws.receive().await,
        }
    }

    async fn open(&self) -> Result<()> {
        match self {
            ClientHttpTransport::Sse(sse) => sse.open().await,
            ClientHttpTransport::Ws(ws) => ws.open().await,
        }
    }

    async fn close(&self) -> Result<()> {
        match self {
            ClientHttpTransport::Sse(sse) => sse.close().await,
            ClientHttpTransport::Ws(ws) => ws.close().await,
        }
    }
}
