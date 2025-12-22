pub mod executors;
pub mod plugin_trait;

pub use distri_types::OrchestratorTrait;
pub use executors::*;
pub use plugin_trait::{PluginContext, PluginExecutor, PluginInfo, PluginItem};
#[cfg(test)]
mod tests;
