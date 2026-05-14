//! Connection resolution — unified path for resolving a connection_id into
//! the materials needed to authenticate a downstream call (env vars + HTTP
//! headers), regardless of whether the connection is OAuth, Custom, or
//! DistriNative.
//!
//! The resolver is the single source of truth used by:
//! - `inject_connection_env` tool (writes to `ExecutorContext.env_vars`)
//! - `x-connection-id` path in `request.rs` (injects `Authorization` header)
//! - the cloud-side `POST /request` proxy handler (same, but server-side)
//! - orchestrator declarative `definition.connections` resolution at run start

pub mod resolver;

pub use resolver::{
    ConnectionResolver, CredentialResolver, DefaultResolver, ResolveCtx, ResolvedConnection,
    ResolvedCredential,
};
