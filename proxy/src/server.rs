#![allow(dead_code)]
use anyhow::Result;
use async_mcp::{
    client::ClientBuilder,
    protocol::RequestOptions,
    server::Server,
    transport::{
        ClientSseTransport, ClientStdioTransport, ClientWsTransport, ClientWsTransportBuilder,
        Message, Transport,
    },
    types::{
        CallToolRequest, CallToolResponse, ListRequest, ResourcesListResponse, ServerCapabilities,
        Tool, ToolResponseContent, ToolsListResponse,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::read_to_string;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

use crate::types::{ProxyMcpServer, ProxyMcpServerType, ProxyServerConfig as Config};

// Update the type to use an enum
#[derive(Clone)]
enum ClientTransport {
    SSE(ClientSseTransport),
    Stdio(ClientStdioTransport),
    WS(ClientWsTransport),
}

const TOOL_SEPARATOR: &str = "---";
#[async_trait::async_trait]
impl Transport for ClientTransport {
    async fn send(&self, message: &Message) -> Result<()> {
        match self {
            ClientTransport::SSE(t) => t.send(message).await,
            ClientTransport::Stdio(t) => t.send(message).await,
            ClientTransport::WS(t) => t.send(message).await,
        }
    }

    async fn receive(&self) -> Result<Option<Message>> {
        match self {
            ClientTransport::SSE(t) => t.receive().await,
            ClientTransport::Stdio(t) => t.receive().await,
            ClientTransport::WS(t) => t.receive().await,
        }
    }

    async fn close(&self) -> Result<()> {
        match self {
            ClientTransport::SSE(t) => t.close().await,
            ClientTransport::Stdio(t) => t.close().await,
            ClientTransport::WS(t) => t.close().await,
        }
    }
    async fn open(&self) -> Result<()> {
        match self {
            ClientTransport::SSE(t) => t.open().await?,
            ClientTransport::Stdio(t) => t.open().await?,
            ClientTransport::WS(t) => t.open().await?,
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct McpProxy {
    config: Arc<Config>,
    clients: Arc<Mutex<HashMap<String, async_mcp::client::Client<ClientTransport>>>>,
    tools_cache: Arc<Mutex<HashMap<String, Vec<Tool>>>>,
    resources_cache: Arc<Mutex<HashMap<String, Vec<async_mcp::types::Resource>>>>,
}

#[derive(Serialize, Deserialize)]
pub struct McpCache {
    tools: HashMap<String, Vec<Tool>>,
    resources: HashMap<String, Vec<async_mcp::types::Resource>>,
}

impl McpProxy {
    /// Initialize the proxy's caches from a file path or a JSON string
    pub fn new(config: Arc<Config>, cached_content: &str) -> Result<McpProxy> {
        let cache_data: McpCache = serde_json::from_str(&cached_content)?;

        // Update the tools cache
        let proxy = McpProxy {
            config,
            clients: Arc::new(Mutex::new(HashMap::new())),
            tools_cache: Arc::new(Mutex::new(cache_data.tools)),
            resources_cache: Arc::new(Mutex::new(cache_data.resources)),
        };

        Ok(proxy)
    }

    pub async fn initialize(config: Arc<Config>) -> Result<Self> {
        info!("Creating new MCP Proxy");
        let proxy = Self {
            config,
            clients: Arc::new(Mutex::new(HashMap::new())),
            tools_cache: Arc::new(Mutex::new(HashMap::new())),
            resources_cache: Arc::new(Mutex::new(HashMap::new())),
        };

        // Initialize caches for all servers
        proxy.init_caches().await?;

        Ok(proxy)
    }

    async fn get_or_create_client(
        &self,
        server_name: &str,
        server: &ProxyMcpServer,
        env_vars: Option<HashMap<String, String>>,
    ) -> Result<async_mcp::client::Client<ClientTransport>> {
        let mut clients = self.clients.lock().await;

        if let Some(client) = clients.get(server_name) {
            return Ok(client.clone());
        }

        let transport = match &server.server_type {
            ProxyMcpServerType::SSE { url, headers } => {
                let mut transport = ClientSseTransport::builder(url.clone());
                let transport = match headers {
                    Some(headers) => {
                        for (key, value) in headers.iter() {
                            transport = transport.with_header(key, value);
                        }
                        transport
                    }
                    None => transport,
                }
                .build();

                ClientTransport::SSE(transport)
            }
            ProxyMcpServerType::Stdio {
                command,
                args,
                env_vars: default_env_vars,
            } => {
                let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                let env_vars = match env_vars {
                    Some(env_vars) => Some(env_vars),
                    None => default_env_vars.clone(),
                };
                let transport = ClientStdioTransport::new(command.as_str(), &args, env_vars)?;
                ClientTransport::Stdio(transport)
            }
            ProxyMcpServerType::WS { url, headers } => {
                let mut transport = ClientWsTransportBuilder::new(url.clone());
                let transport = match headers {
                    Some(headers) => {
                        for (key, value) in headers.iter() {
                            transport = transport.with_header(key, value);
                        }
                        transport
                    }
                    None => transport,
                }
                .build();
                ClientTransport::WS(transport)
            }
        };

        match &transport {
            ClientTransport::SSE(t) => t.open().await?,
            ClientTransport::Stdio(t) => t.open().await?,
            ClientTransport::WS(t) => t.open().await?,
        }

        let client = ClientBuilder::new(transport).build();
        let client_clone = client.clone();

        tokio::spawn(async move { client_clone.start().await });

        clients.insert(server_name.to_string(), client.clone());
        Ok(client)
    }

    pub async fn build<T: Transport>(self, t: T) -> Result<Server<T>> {
        let proxy = Arc::new(self);

        let server = Server::builder(t)
            .capabilities(ServerCapabilities::default())
            .request_handler("resources/list", {
                let proxy = proxy.clone();
                move |_req: ListRequest| {
                    let proxy = proxy.clone();
                    Box::pin(async move { Ok(proxy.aggregate_resources().await) })
                }
            })
            .request_handler("tools/list", {
                let proxy = proxy.clone();
                move |_req: ListRequest| {
                    let proxy = proxy.clone();
                    Box::pin(async move { Ok(proxy.aggregate_tools().await) })
                }
            })
            .request_handler("tools/call", {
                let proxy = proxy.clone();
                move |req: CallToolRequest| {
                    let proxy = proxy.clone();
                    Box::pin(async move {
                        match proxy.handle_tool(req).await {
                            Ok(response) => Ok(response),
                            Err(e) => Ok(CallToolResponse {
                                content: vec![ToolResponseContent::Text {
                                    text: e.to_string(),
                                }],
                                is_error: Some(true),
                                meta: None,
                            }),
                        }
                    })
                }
            });

        Ok(server.build())
    }

    async fn init_caches(&self) -> Result<()> {
        info!("Initializing caches for all servers");
        let mut tool_futures = Vec::new();
        let mut resource_futures = Vec::new();

        // Create futures for all servers
        for (name, server) in &self.config.servers {
            info!("Setting up server: {}", name);
            let name = name.clone();
            let server = server.clone();
            let self_clone = self;

            let name_clone = name.clone();
            let server_clone = server.clone();

            // Future for fetching tools
            let tools_future = async move {
                debug!("Fetching tools for server: {}", name);
                let client = match self_clone.get_or_create_client(&name, &server, None).await {
                    Ok(client) => client,
                    Err(e) => {
                        error!("Failed to connect to server {}: {:?}", name, e);
                        return Ok((name, Vec::new())); // Return empty tools on error
                    }
                };

                debug!("Sending tools/list request to {}", name);
                let response = client
                    .request(
                        "tools/list",
                        None,
                        RequestOptions::default()
                            .timeout(Duration::from_secs(self.config.timeout.list)),
                    )
                    .await?;

                // Parse JSON-RPC response
                debug!("tools/list response  {response}");
                match serde_json::from_value::<serde_json::Value>(response) {
                    Ok(value) => {
                        let tools_response: ToolsListResponse = serde_json::from_value(value)?;
                        info!(
                            "Successfully fetched {} tools from {}",
                            tools_response.tools.len(),
                            name
                        );
                        Ok((name, tools_response.tools))
                    }
                    Err(e) => {
                        error!("Failed to parse tools response from {}: {:?}", name, e);
                        Ok((name, Vec::new()))
                    }
                }
            };
            tool_futures.push(tools_future);

            // Future for fetching resources
            let resources_future = async move {
                debug!("Fetching resources for server: {}", name_clone);
                let client = match self_clone
                    .get_or_create_client(&name_clone, &server_clone, None)
                    .await
                {
                    Ok(client) => client,
                    Err(e) => {
                        error!("Failed to connect to server {}: {:?}", name_clone, e);
                        return Ok((name_clone, Vec::new())); // Return empty resources on error
                    }
                };

                debug!("Sending resources/list request to {}", name_clone);
                let server_resources = match client
                    .request(
                        "resources/list",
                        None,
                        RequestOptions::default()
                            .timeout(Duration::from_secs(self.config.timeout.list)),
                    )
                    .await
                {
                    Ok(response) => match serde_json::from_value::<ResourcesListResponse>(response)
                    {
                        Ok(resources) => resources,
                        Err(e) => {
                            error!("Invalid resources response from {}: {:?}", name_clone, e);
                            return Ok((name_clone, Vec::new())); // Return empty resources on parse error
                        }
                    },
                    Err(e) => {
                        error!("Failed to fetch resources from {}: {:?}", name_clone, e);
                        // Return empty resources on request error
                        return Ok((name_clone, Vec::new()));
                    }
                };

                info!(
                    "Successfully fetched {} resources from {}",
                    server_resources.resources.len(),
                    name_clone
                );
                Ok((name_clone, server_resources.resources))
            };
            resource_futures.push(resources_future);
        }

        info!("Waiting for all servers to respond...");
        let (tools_results, resources_results) = match tokio::try_join!(
            async {
                debug!("Waiting for tools futures");
                let results: Result<Vec<_>> = futures::future::try_join_all(tool_futures).await;
                results
            },
            async {
                debug!("Waiting for resources futures");
                let results: Result<Vec<_>> = futures::future::try_join_all(resource_futures).await;
                results
            }
        ) {
            Ok(results) => results,
            Err(e) => {
                info!("Failed to initialize caches: {:?}", e);
                return Err(e);
            }
        };

        // Update caches with results
        debug!("Updating tools cache");
        let mut tools_cache = self.tools_cache.lock().await;
        *tools_cache = HashMap::new();
        for (name, tools) in tools_results {
            info!("Server {}: Cached {} tools", name, tools.len());
            tools_cache.insert(name, tools);
        }

        debug!("Updating resources cache");
        let mut resources_cache = self.resources_cache.lock().await;
        *resources_cache = HashMap::new();
        for (name, resources) in resources_results {
            info!("Server {}: Cached {} resources", name, resources.len());
            resources_cache.insert(name, resources);
        }

        info!("Successfully initialized all caches");
        Ok(())
    }

    // Rest of the implementation methods...
    async fn aggregate_resources(&self) -> ResourcesListResponse {
        let resources = self.resources_cache.lock().await;
        let mut all_resources = Vec::new();

        for server_resources in resources.values() {
            all_resources.extend_from_slice(server_resources);
        }

        ResourcesListResponse {
            resources: all_resources,
            next_cursor: None,
            meta: None,
        }
    }

    async fn aggregate_tools(&self) -> Value {
        let tools = self.tools_cache.lock().await;
        let mut all_tools = Vec::new();

        for (server_name, server_tools) in tools.iter() {
            for tool in server_tools {
                let mut tool = tool.clone();
                tool.name = format!("{}{TOOL_SEPARATOR}{}", server_name, tool.name);
                all_tools.push(tool);
            }
        }
        let response = ToolsListResponse {
            tools: all_tools,
            next_cursor: None,
            meta: None,
        };

        serde_json::to_value(response).unwrap_or_default()
    }

    fn get_env_vars(req: &CallToolRequest) -> Option<HashMap<String, String>> {
        if let Some(Value::Object(meta)) = req.meta.as_ref() {
            if let Some(Value::Object(vars)) = meta.get("env_vars") {
                let mut env_vars = HashMap::new();
                for (key, value) in vars {
                    if let Value::String(value) = value {
                        env_vars.insert(key.clone(), value.clone());
                    }
                }
                Some(env_vars)
            } else {
                None
            }
        } else {
            None
        }
    }

    async fn handle_tool(&self, req: CallToolRequest) -> Result<CallToolResponse> {
        // Check if server is specified in the request
        let server_name_parts = req.name.split(TOOL_SEPARATOR).collect::<Vec<&str>>();

        if server_name_parts.len() == 2 {
            let server_name = server_name_parts[0];
            let function_name = server_name_parts[1];
            if let Some(server) = self.config.servers.get(server_name) {
                // Extract env_vars from meta if they exist

                let env_vars = Self::get_env_vars(&req);

                if let Ok(client) = self
                    .get_or_create_client(&server_name, server, env_vars)
                    .await
                {
                    let mut req = req.clone();
                    req.name = function_name.to_string();

                    info!("Executing tool {} on server {}", function_name, server_name);
                    debug!("Tool request: {:?}", req);
                    let response = client
                        .request(
                            "tools/call",
                            Some(serde_json::to_value(&req)?),
                            RequestOptions::default()
                                .timeout(Duration::from_secs(self.config.timeout.call)),
                        )
                        .await?;
                    return Ok(serde_json::from_value(response)?);
                }
            }
            anyhow::bail!("Specified server {} not found", server_name);
        }

        // If no server specified, find the first server that has the tool
        let tools = self.tools_cache.lock().await;
        for (server_name, server_tools) in tools.iter() {
            if server_tools.iter().any(|s| req.name == s.name) {
                if let Some(server) = self.config.servers.get(server_name) {
                    let env_vars = Self::get_env_vars(&req);
                    if let Ok(client) = self
                        .get_or_create_client(server_name, server, env_vars)
                        .await
                    {
                        let response = client
                            .request(
                                "tools/call",
                                Some(serde_json::to_value(&req)?),
                                RequestOptions::default()
                                    .timeout(Duration::from_secs(self.config.timeout.call)),
                            )
                            .await?;
                        return Ok(serde_json::from_value(response)?);
                    }
                }
            }
        }

        anyhow::bail!("Tool {} not found in any server", req.name)
    }

    /// Get the current state of the proxy's caches
    ///
    /// # Returns
    /// * `Result<McpCache>` - The current cache state if successful
    pub async fn state(&self) -> Result<McpCache> {
        let tools_cache = self.tools_cache.lock().await;
        let resources_cache = self.resources_cache.lock().await;

        Ok(McpCache {
            tools: tools_cache.clone(),
            resources: resources_cache.clone(),
        })
    }
}
