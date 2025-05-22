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
        CallToolRequest, CallToolResponse, Implementation, ListRequest, ResourcesListResponse,
        ServerCapabilities, Tool, ToolResponseContent, ToolsListResponse,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

use crate::auth::handle_auth;
use crate::types::{ProxyMcpServer, ProxyMcpServerType, ProxyServerConfig as Config};

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case", untagged)]
pub enum ToolsSelection {
    All,
    Selected(Vec<String>),
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct ServerToolsSelection {
    pub server_name: String,
    pub tools: ToolsSelection,
}

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
    pub fn new(
        config: Arc<Config>,
        cached_content: &str,
        mcp_cache_update: Option<&McpCache>,
    ) -> Result<McpProxy> {
        let mut cache_data: McpCache = serde_json::from_str(cached_content)?;

        if let Some(mcp_cache_update) = mcp_cache_update {
            cache_data.tools.extend(mcp_cache_update.tools.clone());
            cache_data
                .resources
                .extend(mcp_cache_update.resources.clone());
        }

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

    fn get_env_hash(server_name: &str, env_vars: Option<&HashMap<String, String>>) -> String {
        if let Some(env_vars) = env_vars {
            let mut hasher = DefaultHasher::new();
            for (key, value) in env_vars.iter() {
                key.hash(&mut hasher);
                value.hash(&mut hasher);
            }
            let env_hash = hasher.finish();
            format!("{}-{:x}", server_name, env_hash)
        } else {
            format!("{}-none", server_name)
        }
    }

    fn replace_vars(arg: &str, env_vars_ref: Option<&HashMap<String, String>>) -> String {
        let mut result = arg.to_string();
        if let Some(env_vars_ref) = env_vars_ref {
            if result.contains("{{") && arg.contains("}}") {
                // Extract all {{...}} patterns and replace them
                let re = regex::Regex::new(r"\{\{([^}]+)\}\}").unwrap();
                for cap in re.captures_iter(arg) {
                    let env_key = &cap[1];
                    if let Some(env_value) = env_vars_ref.get(env_key) {
                        // Replace the pattern with the environment variable value
                        let pattern = format!("{{{{{env_key}}}}}");
                        result = result.replace(&pattern, env_value);
                    }
                }
            }
        }
        result
    }

    async fn get_or_create_client(
        &self,
        server_name: &str,
        server: &ProxyMcpServer,
        mut server_env_vars: Option<HashMap<String, String>>,
    ) -> Result<async_mcp::client::Client<ClientTransport>> {
        let mut clients = self.clients.lock().await;

        let client_key = Self::get_env_hash(server_name, server_env_vars.as_ref());

        if let Some(client) = clients.get(&client_key) {
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
                let mut env_vars_ref = default_env_vars.clone();

                let auth_vars = handle_auth(server, &client_key, server_env_vars.as_ref());

                env_vars_ref.extend(auth_vars.clone());
                match server_env_vars.as_mut() {
                    Some(env_vars) => {
                        env_vars.extend(auth_vars);
                    }
                    None => {
                        server_env_vars = Some(HashMap::new());
                        server_env_vars.as_mut().unwrap().extend(auth_vars);
                    }
                }

                let processed_args: Vec<String> = args
                    .iter()
                    .map(|arg| Self::replace_vars(arg, server_env_vars.as_ref()))
                    .collect();
                let processed_args_refs: Vec<&str> =
                    processed_args.iter().map(|s| s.as_str()).collect();

                let processed_env_vars: HashMap<String, String> = default_env_vars
                    .iter()
                    .map(|(key, value)| {
                        let result = Self::replace_vars(value, server_env_vars.as_ref());
                        (key.to_string(), result)
                    })
                    .collect();

                let transport = ClientStdioTransport::new(
                    command.as_str(),
                    &processed_args_refs,
                    Some(processed_env_vars),
                )?;
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

        client.initialize(Implementation::default()).await?;

        clients.insert(client_key, client.clone());

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
                    Box::pin(async move { Ok(proxy.aggregate_tools(None).await) })
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
                let mut proxied_server: ProxyMcpServer = server.clone();
                if let ProxyMcpServerType::Stdio {
                    ref mut env_vars, ..
                } = &mut proxied_server.server_type
                {
                    for (_, value) in env_vars.iter_mut() {
                        if value.starts_with("{{") && value.ends_with("}}") {
                            *value = "none".to_string();
                        }
                    }
                }
                let client = match self_clone
                    .get_or_create_client(&name, &proxied_server, None)
                    .await
                {
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
                        Some(json!({})),
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
                let mut proxied_server: ProxyMcpServer = server_clone.clone();
                if let ProxyMcpServerType::Stdio {
                    ref mut env_vars, ..
                } = &mut proxied_server.server_type
                {
                    for (_, value) in env_vars.iter_mut() {
                        *value = "none".to_string();
                    }
                }
                let client = match self_clone
                    .get_or_create_client(&name_clone, &proxied_server, None)
                    .await
                {
                    Ok(client) => client,
                    Err(e) => {
                        error!(
                            "Failed to connect to server (during client creation) {}: {:?}",
                            name_clone, e
                        );
                        return Ok((name_clone, Vec::new())); // Return empty resources on error
                    }
                };

                debug!("Sending resources/list request to {}", name_clone);
                let server_resources = match client
                    .request(
                        "resources/list",
                        Some(json!({})),
                        RequestOptions::default()
                            .timeout(Duration::from_secs(self_clone.config.timeout.list)),
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
        let (resources_results, tools_results) = match tokio::try_join!(
            async {
                debug!("Waiting for resources futures");
                let results: Result<Vec<_>> = futures::future::try_join_all(resource_futures).await;
                results
            },
            async {
                debug!("Waiting for tools futures");
                let results: Result<Vec<_>> = futures::future::try_join_all(tool_futures).await;
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
    pub async fn aggregate_resources(&self) -> ResourcesListResponse {
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

    pub async fn aggregate_tools(
        &self,
        tools_selection: Option<Vec<ServerToolsSelection>>,
    ) -> Value {
        let tools = self.tools_cache.lock().await;
        let mut all_tools = Vec::new();

        if let Some(tools_selection) = tools_selection {
            for server_tools_selection in tools_selection {
                if let Some(tools) = tools.get(&server_tools_selection.server_name) {
                    for tool in tools {
                        match &server_tools_selection.tools {
                            ToolsSelection::All => {
                                let mut tool = tool.clone();
                                tool.name = format!(
                                    "{}{TOOL_SEPARATOR}{}",
                                    server_tools_selection.server_name, tool.name
                                );
                                all_tools.push(tool);
                            }
                            ToolsSelection::Selected(tools) => {
                                if tools.contains(&tool.name) {
                                    let mut tool = tool.clone();
                                    tool.name = format!(
                                        "{}{TOOL_SEPARATOR}{}",
                                        server_tools_selection.server_name, tool.name
                                    );
                                    all_tools.push(tool);
                                }
                            }
                        }
                    }
                }
            }
        } else {
            for (server_name, server_tools) in tools.iter() {
                for tool in server_tools {
                    let mut tool = tool.clone();
                    tool.name = format!("{}{TOOL_SEPARATOR}{}", server_name, tool.name);
                    all_tools.push(tool);
                }
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

    pub async fn handle_tool(&self, req: CallToolRequest) -> Result<CallToolResponse> {
        // Check if server is specified in the request
        let server_name_parts = req.name.split(TOOL_SEPARATOR).collect::<Vec<&str>>();

        if server_name_parts.len() == 2 {
            let server_name = server_name_parts[0];
            let function_name = server_name_parts[1];
            let server = if let Some(Value::Object(meta)) = req.meta.as_ref() {
                if let Some(v) = meta.get("dynamic_server") {
                    let dynamic_server: ProxyMcpServerType = serde_json::from_value(v.clone())?;
                    Some(ProxyMcpServer {
                        default_args: None,
                        auth: None,
                        server_type: dynamic_server,
                    })
                } else {
                    self.config.servers.get(server_name).cloned()
                }
            } else {
                self.config.servers.get(server_name).cloned()
            };

            if let Some(server) = &server {
                let env_vars = Self::get_env_vars(&req);

                match self
                    .get_or_create_client(server_name, server, env_vars)
                    .await
                {
                    Ok(client) => {
                        let mut req = req.clone();
                        req.name = function_name.to_string();
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
                    Err(e) => {
                        error!(
                            "Failed to get or create client for server {}: {:?}",
                            server_name, e
                        );
                    }
                }
            }
            anyhow::bail!(
                "Specified server {} not found. Available servers: {}",
                server_name,
                self.config
                    .servers
                    .keys()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>()
                    .join(", ")
            );
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
