//! A2A protocol types for distri — A2A **1.0** (mirrors `a2aproject/a2a-rs`).
//!
//! Anything distri-specific is an A2A extension (see `distri_types::a2a_extensions`);
//! this crate is the protocol only, plus [`legacy`] for the 0.x wire shapes
//! still served to callers of the old method names for one release.

mod agent_card;
mod jsonrpc;
pub mod legacy;
mod types;

pub use agent_card::*;
pub use jsonrpc::*;
pub use types::*;

/// The A2A protocol version distri speaks (`AgentInterface.protocolVersion`,
/// `A2A-Version` header). Not an agent's version.
pub const A2A_VERSION: &str = "1.0";

/// Header (service parameter) naming the protocol version a client speaks.
pub const VERSION_HEADER: &str = "A2A-Version";

/// Header (service parameter) listing the extension URIs a client activates,
/// comma-separated or repeated. The server echoes the ones it activated.
pub const EXTENSIONS_HEADER: &str = "A2A-Extensions";

/// Parse `A2A-Extensions` header values (comma-separated and/or repeated)
/// into a de-duplicated list of URIs, in order.
pub fn parse_extensions_header<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in values {
        for uri in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if !out.iter().any(|u| u == uri) {
                out.push(uri.to_string());
            }
        }
    }
    out
}
