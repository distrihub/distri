//! Observability module for LLM Gateway — OpenTelemetry spans, context, and recording.

pub mod builder;
pub mod context;
pub mod recorder;
pub mod tts;
pub mod types;

pub use context::ContextFields;

pub use builder::*;
pub use recorder::*;
pub use tts::*;
pub use types::*;
