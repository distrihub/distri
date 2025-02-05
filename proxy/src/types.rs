use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct ProxyServerConfig {
    pub servers: HashMap<String, ProxyMcpServer>,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct ProxyMcpServer {
    pub default_args: Option<Value>,
    #[serde(flatten)]
    pub server_type: ProxyMcpServerType,
}
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
#[serde(tag = "type")]
pub enum ProxyMcpServerType {
    #[serde(rename = "stdio")]
    Stdio { command: String, args: Vec<String> },
    #[serde(rename = "sse")]
    SSE {
        url: String,
        auth: Option<ProxyTransportAuth>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "auth_type", content = "value")]
pub enum ProxyTransportAuth {
    Bearer(String),
    JwtSecret(String),
}
