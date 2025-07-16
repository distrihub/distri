/// Macro to automatically implement common BaseAgent methods for custom agents
/// that wrap a StandardAgent
#[macro_export]
macro_rules! delegate_base_agent {
    ($agent_type:ty, $agent_name:expr, $inner_field:ident) => {
        #[async_trait::async_trait]
        impl $crate::agent::BaseAgent for $agent_type {
            fn agent_type(&self) -> $crate::agent::AgentType {
                $crate::agent::AgentType::Custom($agent_name.to_string())
            }

            fn get_definition(&self) -> $crate::types::AgentDefinition {
                self.$inner_field.get_definition()
            }

            fn get_description(&self) -> &str {
                self.$inner_field.get_description()
            }

            fn get_tools(&self) -> Vec<&Box<dyn $crate::tools::Tool>> {
                self.$inner_field.get_tools()
            }

            fn get_name(&self) -> &str {
                self.$inner_field.get_name()
            }

            fn clone_box(&self) -> Box<dyn $crate::agent::BaseAgent> {
                Box::new(self.clone())
            }

            fn get_hooks(&self) -> Option<&dyn $crate::agent::AgentHooks> {
                Some(self)
            }

            async fn invoke(
                &self,
                task: $crate::memory::TaskStep,
                params: Option<serde_json::Value>,
                context: std::sync::Arc<$crate::agent::ExecutorContext>,
                event_tx: Option<tokio::sync::mpsc::Sender<$crate::agent::AgentEvent>>,
            ) -> Result<String, $crate::error::AgentError> {
                self.$inner_field
                    .invoke(task, params, context, event_tx)
                    .await
            }

            async fn invoke_stream(
                &self,
                task: $crate::memory::TaskStep,
                params: Option<serde_json::Value>,
                context: std::sync::Arc<$crate::agent::ExecutorContext>,
                event_tx: tokio::sync::mpsc::Sender<$crate::agent::AgentEvent>,
            ) -> Result<(), $crate::error::AgentError> {
                self.$inner_field
                    .invoke_stream(task, params, context, event_tx)
                    .await
            }
        }
    };
}

/// Macro to create a custom agent with automatic BaseAgent implementation
#[macro_export]
macro_rules! custom_agent {
    (
        $agent_name:ident,
        $agent_type_name:expr,
        $inner_field:ident,
        $custom_fields:tt
    ) => {
        #[derive(Clone)]
        pub struct $agent_name {
            $inner_field: $crate::agent::StandardAgent,
            $custom_fields
        }

        impl std::fmt::Debug for $agent_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($agent_name))
                    .field(stringify!($inner_field), &self.$inner_field)
                    .finish()
            }
        }

        impl $agent_name {
            pub fn new(
                definition: $crate::types::AgentDefinition,
                tools_registry: std::sync::Arc<$crate::tools::LlmToolsRegistry>,
                coordinator: std::sync::Arc<$crate::agent::AgentExecutor>,
                context: std::sync::Arc<$crate::agent::ExecutorContext>,
                session_store: std::sync::Arc<Box<dyn $crate::SessionStore>>,
            ) -> Self {
                let $inner_field = $crate::agent::StandardAgent::new(
                    definition,
                    tools_registry,
                    coordinator,
                    context,
                    session_store,
                );
                Self {
                    $inner_field,
                    // Initialize custom fields here
                }
            }
        }

        $crate::impl_base_agent_delegate!($agent_name, $agent_type_name, $inner_field);
    };
}
