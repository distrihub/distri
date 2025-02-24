use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct ProxyServerConfig {
    pub servers: HashMap<String, ProxyMcpServer>,
    pub port: u16,
    #[serde(default)]
    pub timeout: TimeoutConfig,
}
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct TimeoutConfig {
    #[serde(default = "default_list_timeout")]
    pub list: u64,
    #[serde(default = "default_call_timeout")]
    pub call: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            list: default_list_timeout(),
            call: default_call_timeout(),
        }
    }
}

fn default_list_timeout() -> u64 {
    120
}
fn default_call_timeout() -> u64 {
    60
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
    Stdio {
        command: String,
        args: Vec<String>,
        #[serde(flatten, skip_serializing_if = "Option::is_none")]
        env_vars: Option<HashMap<String, String>>,
    },
    #[serde(rename = "sse")]
    SSE {
        url: String,
        headers: Option<HashMap<String, String>>,
    },
    #[serde(rename = "ws")]
    WS {
        url: String,
        headers: Option<HashMap<String, String>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "auth_type", content = "value")]
pub enum ProxyTransportAuth {
    Bearer(String),
    JwtSecret(String),
}
