// Authentication implementation crate for Distri
// Contains implementations for auth providers, session management, and stores

// pub mod auth_server; // Deprecated - use oauth_handler + cli_auth_server instead
pub mod auth_routes;
pub mod cli_commands;
pub mod context;
pub mod file_store;
pub mod implementations;
pub mod provider_registry;
pub mod provider_session_store;
pub mod providers;

// Re-export commonly used types and traits from distri-types
pub use distri_types::auth::{
    AuthProvider, AuthSecret, AuthSession, AuthType, OAuth2FlowType, OAuth2State, OAuthHandler,
    ProviderRegistry as BaseProviderRegistry,
};

// Re-export implementations
pub use cli_auth_server::oauth_handler::{OAuthCallback, OAuthHandlerState, OAuthStartParams};
pub use cli_auth_server::{oauth_handler, CallbackConfig, CliAuthServer};
pub use cli_commands::*;
pub use context::{ToolAuthRequestContext, UserContext};
pub use file_store::FileToolAuthStore;
pub use implementations::*;
pub use provider_registry::{ProviderConfig, ProviderRegistry, ProvidersConfig};
pub use provider_session_store::*;
pub use providers::*;

const LOCAL_USER_ID: &str = "0d6a4a55-e992-4888-874a-1ed7c66613e5";

pub fn get_local_user_id() -> String {
    return LOCAL_USER_ID.to_string();
}

pub fn default_auth_providers() -> String {
    include_str!("./providers/default_providers.json").to_string()
}
