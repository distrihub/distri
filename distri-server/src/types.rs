use a2a_rs::types::AgentCard;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentResponse {
    pub agents: Vec<AgentCard>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: u32,
}

/// Represents an A2A Agent Card as per the specification
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentCard {
    /// The unique identifier for the agent
    pub id: String,
    /// The name of the agent
    pub name: String,
    /// A description of the agent's capabilities
    pub description: String,
    /// The version of the agent
    pub version: String,
    /// The capabilities of the agent
    pub capabilities: Vec<Capability>,
    /// The skills of the agent
    pub skills: Vec<Skill>,
    /// The authentication methods supported by the agent
    pub auth_methods: Vec<AuthMethod>,
    /// The endpoints where the agent can be reached
    pub endpoints: Vec<Endpoint>,
    /// Additional metadata about the agent
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Capability {
    /// The name of the capability
    pub name: String,
    /// A description of the capability
    pub description: String,
    /// The parameters required for this capability
    #[serde(default)]
    pub parameters: HashMap<String, Parameter>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Skill {
    /// The name of the skill
    pub name: String,
    /// A description of the skill
    pub description: String,
    /// The capabilities required for this skill
    pub required_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthMethod {
    /// The type of authentication method
    pub r#type: String,
    /// The parameters required for this authentication method
    #[serde(default)]
    pub parameters: HashMap<String, Parameter>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Endpoint {
    /// The URL of the endpoint
    pub url: String,
    /// The protocol used by the endpoint
    pub protocol: String,
    /// The authentication methods supported by this endpoint
    pub auth_methods: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Parameter {
    /// The type of the parameter
    pub r#type: String,
    /// A description of the parameter
    pub description: String,
    /// Whether the parameter is required
    #[serde(default)]
    pub required: bool,
}

/// Trait for converting between our types and A2A types
pub trait ToA2A {
    type A2AType;
    fn to_a2a(&self) -> Self::A2AType;
}

/// Trait for converting from A2A types to our types
pub trait FromA2A {
    type A2AType;
    fn from_a2a(a2a: Self::A2AType) -> Self;
}
