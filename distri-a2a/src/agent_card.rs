//! A2A 1.0 Agent Card. Mirrors `a2a-rs` `a2a/src/agent_card.rs`.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::types::Metadata;

/// JSON-RPC binding name for [`AgentInterface::protocol_binding`].
pub const TRANSPORT_PROTOCOL_JSONRPC: &str = "JSONRPC";
/// HTTP+JSON (REST) binding name.
pub const TRANSPORT_PROTOCOL_HTTP_JSON: &str = "HTTP+JSON";
/// gRPC binding name.
pub const TRANSPORT_PROTOCOL_GRPC: &str = "GRPC";

/// A self-describing manifest for an agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The agent's own version (not the protocol version).
    pub version: String,
    /// Where and how the agent is reached; the first entry is preferred.
    /// Each carries the protocol version it speaks.
    pub supported_interfaces: Vec<AgentInterface>,
    pub capabilities: AgentCapabilities,
    #[serde(default)]
    pub default_input_modes: Vec<String>,
    #[serde(default)]
    pub default_output_modes: Vec<String>,
    #[serde(default)]
    pub skills: Vec<AgentSkill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub security_schemes: BTreeMap<String, SecurityScheme>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_requirements: Vec<SecurityRequirement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signatures: Vec<AgentCardSignature>,
}

impl AgentCard {
    /// The URL of the preferred interface.
    pub fn url(&self) -> Option<&str> {
        self.supported_interfaces.first().map(|i| i.url.as_str())
    }

    /// The protocol version of the preferred interface.
    pub fn protocol_version(&self) -> Option<&str> {
        self.supported_interfaces
            .first()
            .map(|i| i.protocol_version.as_str())
    }

    /// Whether the card declares an extension by URI.
    pub fn has_extension(&self, uri: &str) -> bool {
        self.capabilities.extensions.iter().any(|e| e.uri == uri)
    }
}

/// A URL + protocol binding + protocol version the agent serves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentInterface {
    pub url: String,
    pub protocol_binding: String,
    pub protocol_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

impl AgentInterface {
    /// A JSON-RPC interface speaking [`crate::A2A_VERSION`].
    pub fn jsonrpc(url: impl Into<String>) -> Self {
        AgentInterface {
            url: url.into(),
            protocol_binding: TRANSPORT_PROTOCOL_JSONRPC.to_string(),
            protocol_version: crate::A2A_VERSION.to_string(),
            tenant: None,
        }
    }
}

/// Who provides the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentProvider {
    pub organization: String,
    pub url: String,
}

/// Optional capabilities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_notifications: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<AgentExtension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extended_agent_card: Option<bool>,
}

/// A protocol extension the agent supports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentExtension {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The client must activate it (else `-32008`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Metadata>,
}

/// A skill the agent offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_modes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_modes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(with = "Vec<serde_json::Value>")]
    pub security_requirements: Vec<SecurityRequirement>,
}

/// A JWS signature over the card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentCardSignature {
    pub protected: String,
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<Metadata>,
}

/// Scheme name → required scopes. Wire: `{"schemes": {name: {"list": [..]}}}`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SecurityRequirement(pub BTreeMap<String, Vec<String>>);

impl Serialize for SecurityRequirement {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let schemes: BTreeMap<&str, Value> = self
            .0
            .iter()
            .map(|(k, v)| (k.as_str(), serde_json::json!({ "list": v })))
            .collect();
        serde_json::json!({ "schemes": schemes }).serialize(s)
    }
}

impl<'de> Deserialize<'de> for SecurityRequirement {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        // 1.0 `{schemes: {name: {list}}}` or 0.x `{name: [scopes]}`.
        let map = match v.get("schemes") {
            Some(Value::Object(m)) => m.clone(),
            _ => v.as_object().cloned().unwrap_or_default(),
        };
        let mut out = BTreeMap::new();
        for (k, scopes) in map {
            let list = match scopes {
                Value::Array(_) => scopes,
                Value::Object(mut o) => o.remove("list").unwrap_or(Value::Array(vec![])),
                _ => Value::Array(vec![]),
            };
            out.insert(
                k,
                serde_json::from_value(list).map_err(serde::de::Error::custom)?,
            );
        }
        Ok(SecurityRequirement(out))
    }
}

/// A security scheme (field-presence union).
#[derive(Debug, Clone, PartialEq)]
pub enum SecurityScheme {
    ApiKey(ApiKeySecurityScheme),
    HttpAuth(HttpAuthSecurityScheme),
    OAuth2(Box<OAuth2SecurityScheme>),
    OpenIdConnect(OpenIdConnectSecurityScheme),
    MutualTls(MutualTlsSecurityScheme),
}

impl Serialize for SecurityScheme {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            SecurityScheme::ApiKey(s) => map.serialize_entry("apiKeySecurityScheme", s)?,
            SecurityScheme::HttpAuth(s) => map.serialize_entry("httpAuthSecurityScheme", s)?,
            SecurityScheme::OAuth2(s) => map.serialize_entry("oauth2SecurityScheme", s)?,
            SecurityScheme::OpenIdConnect(s) => {
                map.serialize_entry("openIdConnectSecurityScheme", s)?
            }
            SecurityScheme::MutualTls(s) => map.serialize_entry("mtlsSecurityScheme", s)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for SecurityScheme {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut raw = Metadata::deserialize(deserializer)?;
        if let Some(v) = raw.remove("apiKeySecurityScheme") {
            return Ok(SecurityScheme::ApiKey(de::<_, D::Error>(v)?));
        }
        if let Some(v) = raw.remove("httpAuthSecurityScheme") {
            return Ok(SecurityScheme::HttpAuth(de::<_, D::Error>(v)?));
        }
        if let Some(v) = raw.remove("oauth2SecurityScheme") {
            return Ok(SecurityScheme::OAuth2(Box::new(de::<_, D::Error>(v)?)));
        }
        if let Some(v) = raw.remove("openIdConnectSecurityScheme") {
            return Ok(SecurityScheme::OpenIdConnect(de::<_, D::Error>(v)?));
        }
        if let Some(v) = raw.remove("mtlsSecurityScheme") {
            return Ok(SecurityScheme::MutualTls(de::<_, D::Error>(v)?));
        }
        // 0.x / OpenAPI shape: `{"type": "apiKey" | "http" | "oauth2" |
        // "openIdConnect" | "mutualTLS", ...}` (agent configs saved before 1.0).
        let ty = raw
            .remove("type")
            .and_then(|t| t.as_str().map(String::from));
        let desc = raw
            .get("description")
            .and_then(Value::as_str)
            .map(String::from);
        let get = |k: &str| raw.get(k).and_then(Value::as_str).map(String::from);
        match ty.as_deref() {
            Some("apiKey") => Ok(SecurityScheme::ApiKey(ApiKeySecurityScheme {
                location: get("in").unwrap_or_else(|| "header".into()),
                name: get("name").unwrap_or_default(),
                description: desc,
            })),
            Some("http") => Ok(SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                scheme: get("scheme").unwrap_or_default(),
                bearer_format: get("bearerFormat"),
                description: desc,
            })),
            Some("openIdConnect") => {
                Ok(SecurityScheme::OpenIdConnect(OpenIdConnectSecurityScheme {
                    open_id_connect_url: get("openIdConnectUrl").unwrap_or_default(),
                    description: desc,
                }))
            }
            Some("mutualTLS") => Ok(SecurityScheme::MutualTls(MutualTlsSecurityScheme {
                description: desc,
            })),
            Some("oauth2") => {
                let flows = raw.get("flows").cloned().unwrap_or(Value::Null);
                let flows: OAuthFlows = de::<_, D::Error>(flows)?;
                Ok(SecurityScheme::OAuth2(Box::new(OAuth2SecurityScheme {
                    flows,
                    description: desc,
                    oauth2_metadata_url: None,
                })))
            }
            _ => Err(serde::de::Error::custom("unknown security scheme variant")),
        }
    }
}

/// API key in a header, query or cookie.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeySecurityScheme {
    /// `header`, `query` or `cookie`.
    pub location: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// HTTP authentication (Bearer, Basic, …).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HttpAuthSecurityScheme {
    pub scheme: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// OAuth 2.0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuth2SecurityScheme {
    pub flows: OAuthFlows,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth2_metadata_url: Option<String>,
}

/// OpenID Connect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpenIdConnectSecurityScheme {
    pub open_id_connect_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Mutual TLS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MutualTlsSecurityScheme {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One OAuth 2.0 flow (field-presence union).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OAuthFlows {
    AuthorizationCode(AuthorizationCodeOAuthFlow),
    ClientCredentials(ClientCredentialsOAuthFlow),
    DeviceCode(DeviceCodeOAuthFlow),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationCodeOAuthFlow {
    pub authorization_url: String,
    pub token_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_url: Option<String>,
    #[serde(default)]
    pub scopes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pkce_required: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClientCredentialsOAuthFlow {
    pub token_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_url: Option<String>,
    #[serde(default)]
    pub scopes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCodeOAuthFlow {
    pub device_authorization_url: String,
    pub token_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_url: Option<String>,
    #[serde(default)]
    pub scopes: BTreeMap<String, String>,
}

fn de<T: serde::de::DeserializeOwned, E: serde::de::Error>(v: Value) -> Result<T, E> {
    serde_json::from_value(v).map_err(E::custom)
}
