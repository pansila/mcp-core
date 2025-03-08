use crate::protocol::{Protocol, ProtocolBuilder, RequestOptions};
use crate::transport::{
    JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, Message, RequestId,
    Transport,
};
use crate::types::ErrorCode;
use anyhow::Result;
use async_trait::async_trait;
use std::future::Future;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::debug;

/// ClientStdioTransport launches a child process and communicates with it via stdio.
#[derive(Clone)]
pub struct ClientStdioTransport {
    protocol: Protocol,
    stdin: Arc<Mutex<Option<BufWriter<std::process::ChildStdin>>>>,
    stdout: Arc<Mutex<Option<BufReader<std::process::ChildStdout>>>>,
    child: Arc<Mutex<Option<std::process::Child>>>,
    program: String,
    args: Vec<String>,
}

impl ClientStdioTransport {
    pub fn new(program: &str, args: &[&str]) -> Result<Self> {
        Ok(ClientStdioTransport {
            protocol: ProtocolBuilder::new().build(),
            stdin: Arc::new(Mutex::new(None)),
            stdout: Arc::new(Mutex::new(None)),
            child: Arc::new(Mutex::new(None)),
            program: program.to_string(),
            args: args.iter().map(|&s| s.to_string()).collect(),
        })
    }
}

#[async_trait()]
impl Transport for ClientStdioTransport {
    async fn open(&self) -> Result<()> {
        debug!("ClientStdioTransport: Opening transport");
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Child process stdin not available"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Child process stdout not available"))?;

        {
            let mut stdin_lock = self.stdin.lock().await;
            *stdin_lock = Some(BufWriter::new(stdin));
        }
        {
            let mut stdout_lock = self.stdout.lock().await;
            *stdout_lock = Some(BufReader::new(stdout));
        }
        {
            let mut child_lock = self.child.lock().await;
            *child_lock = Some(child);
        }

        // Spawn a background task to continuously poll messages.
        let transport_clone = self.clone();
        tokio::spawn(async move {
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
                    Ok(None) => break, // EOF encountered.
                    Err(e) => {
                        debug!("ClientStdioTransport: Error polling message: {:?}", e);
                        break;
                    }
                }
            }
        });
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        let mut child_lock = self.child.lock().await;
        if let Some(child) = child_lock.as_mut() {
            let _ = child.kill();
        }
        *child_lock = None;

        // Clear stdin and stdout
        *self.stdin.lock().await = None;
        *self.stdout.lock().await = None;

        Ok(())
    }

    async fn poll_message(&self) -> Result<Option<Message>> {
        debug!("ClientStdioTransport: Starting to receive message");

        // Take ownership of stdout temporarily
        let mut stdout_guard = self.stdout.lock().await;
        let mut stdout = stdout_guard
            .take()
            .ok_or_else(|| anyhow::anyhow!("Transport not opened"))?;

        // Drop the lock before spawning the blocking task
        drop(stdout_guard);

        // Use a blocking operation in a spawn_blocking task
        let (line_result, stdout) = tokio::task::spawn_blocking(move || {
            let mut line = String::new();
            let result = match stdout.read_line(&mut line) {
                Ok(0) => Ok(None), // EOF
                Ok(_) => Ok(Some(line)),
                Err(e) => Err(anyhow::anyhow!("Error reading line: {}", e)),
            };
            // Return both the result and the stdout so we can put it back
            (result, stdout)
        })
        .await?;

        // Put stdout back
        let mut stdout_guard = self.stdout.lock().await;
        *stdout_guard = Some(stdout);

        // Process the result
        match line_result? {
            Some(line) => {
                debug!(
                    "ClientStdioTransport: Received from process: {}",
                    line.trim()
                );
                let message: Message = serde_json::from_str(&line)?;
                debug!("ClientStdioTransport: Successfully parsed message");
                Ok(Some(message))
            }
            None => {
                debug!("ClientStdioTransport: Received EOF from process");
                Ok(None)
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
        let stdin_arc = self.stdin.clone();
        let method = method.to_owned();
        Box::pin(async move {
            let (id, rx) = protocol.create_request().await;
            let request = JsonRpcRequest {
                id,
                method,
                jsonrpc: Default::default(),
                params,
            };
            let serialized = serde_json::to_string(&request)?;
            debug!("ClientStdioTransport: Sending request: {}", serialized);

            // Get the stdin writer
            let mut stdin_guard = stdin_arc.lock().await;
            let mut stdin = stdin_guard
                .take()
                .ok_or_else(|| anyhow::anyhow!("Transport not opened"))?;

            // Use a blocking operation in a spawn_blocking task
            let stdin_result = tokio::task::spawn_blocking(move || {
                stdin.write_all(serialized.as_bytes())?;
                stdin.write_all(b"\n")?;
                stdin.flush()?;
                Ok::<_, anyhow::Error>(stdin)
            })
            .await??;

            // Put the writer back
            *stdin_guard = Some(stdin_result);

            debug!("ClientStdioTransport: Request sent successfully");
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
        let serialized = serde_json::to_string(&response)?;
        debug!("ClientStdioTransport: Sending response: {}", serialized);

        // Get the stdin writer
        let mut stdin_guard = self.stdin.lock().await;
        let mut stdin = stdin_guard
            .take()
            .ok_or_else(|| anyhow::anyhow!("Transport not opened"))?;

        // Use a blocking operation in a spawn_blocking task
        let stdin_result = tokio::task::spawn_blocking(move || {
            stdin.write_all(serialized.as_bytes())?;
            stdin.write_all(b"\n")?;
            stdin.flush()?;
            Ok::<_, anyhow::Error>(stdin)
        })
        .await??;

        // Put the writer back
        *stdin_guard = Some(stdin_result);

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
        let serialized = serde_json::to_string(&notification)?;
        debug!("ClientStdioTransport: Sending notification: {}", serialized);

        // Get the stdin writer
        let mut stdin_guard = self.stdin.lock().await;
        let mut stdin = stdin_guard
            .take()
            .ok_or_else(|| anyhow::anyhow!("Transport not opened"))?;

        // Use a blocking operation in a spawn_blocking task
        let stdin_result = tokio::task::spawn_blocking(move || {
            stdin.write_all(serialized.as_bytes())?;
            stdin.write_all(b"\n")?;
            stdin.flush()?;
            Ok::<_, anyhow::Error>(stdin)
        })
        .await??;

        // Put the writer back
        *stdin_guard = Some(stdin_result);

        Ok(())
    }
}
