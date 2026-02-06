mod client;
mod client_app;
mod client_stream;
pub mod config;
mod local_tools;
mod printer;

use thiserror::Error;

pub use crate::external_tools_runtime::ExternalToolRegistry;
pub use crate::hooks_runtime::HookRegistry;
pub use client::{
    AgentRegistrationResponse, ApiKeyResponse, ArtifactEntry, ArtifactListResponse,
    ArtifactNamespace, ArtifactNamespaceList, ArtifactReadResponse, ArtifactSaveResponse,
    CreatePluginRequest, Distri, InvokeOptions, LlmExecuteOptions, LlmExecuteResponse,
    LoginUrlResponse, NewPromptTemplateRequest, PluginResponse, PluginsListResponse,
    PromptTemplateResponse, SyncPromptTemplatesResponse, TaskNamespaceResponse,
    UpdatePluginRequest, ValidatePluginResponse, WorkspaceResponse,
};
pub use client_app::{AppError, DistriClientApp, ToolListItem};
pub use client_stream::{AgentStreamClient, StreamError, StreamItem};
pub use config::{BuildHttpClient, DistriConfig};
pub use hooks_runtime::*;

pub use distri_types::{
    HookContext, HookKind, HookMutation, InlineHookRequest, InlineHookResponse, TokenResponse,
};
pub use local_tools::register_local_filesystem_tools;
pub use printer::{EventPrinter, print_stream};

#[cfg(test)]
mod tests;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("http request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("stdout transport failed: {0}")]
    Stdout(String),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

mod external_tools_runtime;
mod hooks_runtime;

pub use distri_types as types;
