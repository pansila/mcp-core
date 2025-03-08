use std::{collections::HashMap, env, sync::Arc};

use crate::{
    protocol::RequestOptions,
    transport::Transport,
    types::{
        CallToolRequest, CallToolResponse, ClientCapabilities, Implementation, InitializeRequest,
        InitializeResponse, ListRequest, ReadResourceRequest, Resource, ResourcesListResponse,
        ToolsListResponse, LATEST_PROTOCOL_VERSION,
    },
};

use anyhow::Result;
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::debug;

#[derive(Clone)]
pub struct Client<T: Transport> {
    transport: T,
    strict: bool,
    initialize_res: Arc<RwLock<Option<InitializeResponse>>>,
    env: Option<HashMap<String, SecureValue>>,
}

impl<T: Transport> Client<T> {
    pub fn builder(transport: T) -> ClientBuilder<T> {
        ClientBuilder::new(transport)
    }

    pub async fn open(&self) -> Result<()> {
        self.transport.open().await
    }

    pub async fn initialize(
        &self,
        client_info: Implementation,
        capabilities: ClientCapabilities,
    ) -> Result<InitializeResponse> {
        let request = InitializeRequest {
            protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
            capabilities,
            client_info,
        };
        let response = self
            .request(
                "initialize",
                Some(serde_json::to_value(request)?),
                RequestOptions::default(),
            )
            .await?;
        let response: InitializeResponse = serde_json::from_value(response)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {}", e))?;

        if response.protocol_version != LATEST_PROTOCOL_VERSION {
            return Err(anyhow::anyhow!(
                "Unsupported protocol version: {}",
                response.protocol_version
            ));
        }

        // Save the response for later use
        let mut writer = self.initialize_res.write().await;
        *writer = Some(response.clone());

        debug!(
            "Initialized with protocol version: {}",
            response.protocol_version
        );
        self.transport
            .send_notification("notifications/initialized", None)
            .await?;

        Ok(response)
    }

    pub async fn assert_initialized(&self) -> Result<(), anyhow::Error> {
        let reader = self.initialize_res.read().await;
        match &*reader {
            Some(_) => Ok(()),
            None => Err(anyhow::anyhow!("Not initialized")),
        }
    }

    pub async fn request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
        options: RequestOptions,
    ) -> Result<serde_json::Value> {
        let response = self.transport.request(method, params, options).await?;
        response
            .result
            .ok_or_else(|| anyhow::anyhow!("Request failed: {:?}", response.error))
    }

    pub async fn list_tools(
        &self,
        cursor: Option<String>,
        request_options: Option<RequestOptions>,
    ) -> Result<ToolsListResponse> {
        if self.strict {
            self.assert_initialized().await?;
        }

        let list_request = ListRequest { cursor, meta: None };

        let response = self
            .request(
                "tools/list",
                Some(serde_json::to_value(list_request)?),
                request_options.unwrap_or_else(RequestOptions::default),
            )
            .await?;

        Ok(serde_json::from_value(response)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {}", e))?)
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguements: Option<serde_json::Value>,
    ) -> Result<CallToolResponse> {
        if self.strict {
            self.assert_initialized().await?;
        }

        let arguments = if let Some(env) = &self.env {
            arguements
                .as_ref()
                .map(|args| apply_secure_replacements(args, env))
        } else {
            arguements
        };

        let arguments =
            arguments.map(|value| serde_json::from_value(value).unwrap_or_else(|_| HashMap::new()));

        let request = CallToolRequest {
            name: name.to_string(),
            arguments,
            meta: None,
        };

        let response = self
            .request(
                "tools/call",
                Some(serde_json::to_value(request)?),
                RequestOptions::default(),
            )
            .await?;

        Ok(serde_json::from_value(response)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {}", e))?)
    }

    pub async fn list_resources(
        &self,
        cursor: Option<String>,
        request_options: Option<RequestOptions>,
    ) -> Result<ResourcesListResponse> {
        if self.strict {
            self.assert_initialized().await?;
        }

        let list_request = ListRequest { cursor, meta: None };

        let response = self
            .request(
                "resources/list",
                Some(serde_json::to_value(list_request)?),
                request_options.unwrap_or_else(RequestOptions::default),
            )
            .await?;

        Ok(serde_json::from_value(response)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {}", e))?)
    }

    pub async fn read_resource(&self, uri: url::Url) -> Result<Resource> {
        if self.strict {
            self.assert_initialized().await?;
        }

        let read_request = ReadResourceRequest { uri };

        let response = self
            .request(
                "resources/read",
                Some(serde_json::to_value(read_request)?),
                RequestOptions::default(),
            )
            .await?;

        Ok(serde_json::from_value(response)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {}", e))?)
    }

    pub async fn subscribe_to_resource(&self, uri: url::Url) -> Result<()> {
        if self.strict {
            self.assert_initialized().await?;
        }

        let subscribe_request = ReadResourceRequest { uri };

        self.request(
            "resources/subscribe",
            Some(serde_json::to_value(subscribe_request)?),
            RequestOptions::default(),
        )
        .await?;

        Ok(())
    }

    pub async fn unsubscribe_to_resource(&self, uri: url::Url) -> Result<()> {
        if self.strict {
            self.assert_initialized().await?;
        }

        let unsubscribe_request = ReadResourceRequest { uri };

        self.request(
            "resources/unsubscribe",
            Some(serde_json::to_value(unsubscribe_request)?),
            RequestOptions::default(),
        )
        .await?;

        Ok(())
    }
}

#[derive(Clone)]
pub enum SecureValue {
    Static(String),
    Env(String),
}

pub struct ClientBuilder<T: Transport> {
    transport: T,
    strict: bool,
    env: Option<HashMap<String, SecureValue>>,
}

impl<T: Transport> ClientBuilder<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            strict: false,
            env: None,
        }
    }

    pub fn with_secure_value(mut self, key: impl Into<String>, value: SecureValue) -> Self {
        match &mut self.env {
            Some(env) => {
                env.insert(key.into(), value);
            }
            None => {
                let mut new_env = HashMap::new();
                new_env.insert(key.into(), value);
                self.env = Some(new_env);
            }
        }
        self
    }

    pub fn use_strict(mut self) -> Self {
        self.strict = true;
        self
    }

    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    pub fn build(self) -> Client<T> {
        Client {
            transport: self.transport,
            strict: self.strict,
            env: self.env,
            initialize_res: Arc::new(RwLock::new(None)),
        }
    }
}

/// Recursively walk through the JSON value. If a JSON string exactly matches
/// one of the keys in the secure values map, replace it with the corresponding secure value.
pub fn apply_secure_replacements(
    value: &Value,
    secure_values: &HashMap<String, SecureValue>,
) -> Value {
    match value {
        Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map.iter() {
                let new_value = if let Value::String(_) = v {
                    if let Some(secure_val) = secure_values.get(k) {
                        let replacement = match secure_val {
                            SecureValue::Static(val) => val.clone(),
                            SecureValue::Env(env_key) => env::var(env_key)
                                .unwrap_or_else(|_| v.as_str().unwrap().to_string()),
                        };
                        Value::String(replacement)
                    } else {
                        apply_secure_replacements(v, secure_values)
                    }
                } else {
                    apply_secure_replacements(v, secure_values)
                };
                new_map.insert(k.clone(), new_value);
            }
            Value::Object(new_map)
        }
        Value::Array(arr) => {
            let new_arr: Vec<Value> = arr
                .iter()
                .map(|v| apply_secure_replacements(v, secure_values))
                .collect();
            Value::Array(new_arr)
        }
        _ => value.clone(),
    }
}
