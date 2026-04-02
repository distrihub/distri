//! Builds the `distri_request` dynamic tool factory from client config.
//!
//! Any client (CLI, SDK, gateway) that talks to a distri agent can call
//! `build_distri_request_factory` to create a `DynamicToolFactory` that
//! the agent will use to call back into the platform API.  The factory
//! is sent as a `DefinitionOverrides.dynamic_tools` entry in the A2A
//! metadata so the server injects it at agent-config level.

use std::collections::HashMap;

use distri_types::DistriConfig;
use distri_types::configuration::DefinitionOverrides;
use distri_types::dynamic_tool::DynamicToolFactory;
use distri_types::http_request::HttpFactoryConfig;

/// Build a `DynamicToolFactory` named `distri_request` from the client's
/// connection config.  The factory is an HTTP tool whose base_url and auth
/// headers mirror the client's own credentials so the agent can call the
/// platform API on behalf of the logged-in user.
pub fn build_distri_request_factory(config: &DistriConfig) -> DynamicToolFactory {
    let mut headers = HashMap::new();
    if let Some(ref api_key) = config.api_key {
        headers.insert("x-api-key".to_string(), api_key.clone());
    }
    if let Some(ref workspace_id) = config.workspace_id {
        headers.insert("x-workspace-id".to_string(), workspace_id.clone());
    }

    let factory_config = HttpFactoryConfig {
        base_url: config.base_url.clone(),
        headers,
    };

    DynamicToolFactory {
        name: "distri_request".to_string(),
        factory_type: "http".to_string(),
        config: serde_json::to_value(factory_config).expect("HttpFactoryConfig serialization"),
        description: Some(
            "Call the Distri platform REST API. Input: {path, method, headers?, body?}".to_string(),
        ),
    }
}

/// Build `DefinitionOverrides` containing the `distri_request` dynamic tool.
pub fn build_platform_overrides(config: &DistriConfig) -> DefinitionOverrides {
    DefinitionOverrides::new().with_dynamic_tools(vec![build_distri_request_factory(config)])
}

/// Build a `distri_request` factory with an explicit workspace ID override.
/// Used by the gateway which handles requests for different workspaces.
pub fn build_distri_request_factory_for_workspace(
    config: &DistriConfig,
    workspace_id: uuid::Uuid,
) -> DynamicToolFactory {
    let mut headers = HashMap::new();
    if let Some(ref api_key) = config.api_key {
        headers.insert("x-api-key".to_string(), api_key.clone());
    }
    // Always set the workspace header to the specific workspace
    headers.insert("x-workspace-id".to_string(), workspace_id.to_string());

    let factory_config = HttpFactoryConfig {
        base_url: config.base_url.clone(),
        headers,
    };

    DynamicToolFactory {
        name: "distri_request".to_string(),
        factory_type: "http".to_string(),
        config: serde_json::to_value(factory_config).expect("HttpFactoryConfig serialization"),
        description: Some(
            "Call the Distri platform REST API. Input: {path, method, headers?, body?}".to_string(),
        ),
    }
}
