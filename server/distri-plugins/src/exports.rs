use serde::{Deserialize, Serialize};

/// Information extracted from DistriPlugin structure
#[derive(Debug, Serialize, Deserialize)]
pub struct DistriPluginInfo {
    pub tools: Vec<DistriToolInfo>,
    pub workflows: Vec<DistriWorkflowInfo>,
}

/// Tool information extracted from DistriPlugin
#[derive(Debug, Serialize, Deserialize)]
pub struct DistriToolInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub parameters: Option<serde_json::Value>,
}

/// Workflow information extracted from DistriPlugin  
#[derive(Debug, Serialize, Deserialize)]
pub struct DistriWorkflowInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub parameters: Option<serde_json::Value>,
}
