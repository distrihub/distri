//! Observability module for LLM Gateway — OpenTelemetry spans, context, and recording.

pub mod types;
pub mod builder;
pub mod recorder;
pub mod tts;

pub use types::*;
pub use builder::*;
pub use recorder::*;
pub use tts::*;
