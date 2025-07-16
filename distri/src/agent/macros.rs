/// Macro to automatically implement common BaseAgent methods for custom agents
/// that wrap a StandardAgent
#[macro_export]
macro_rules! delegate_base_agent {
    ($agent_type:ty, $agent_name:expr, $inner_field:ident) => {
        impl $agent_type {
            pub fn get_hooks(&self) -> Option<&dyn $crate::agent::AgentHooks> {
                Some(self)
            }
        }

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

            async fn invoke(
                &self,
                message: $crate::types::Message,
                context: std::sync::Arc<$crate::agent::ExecutorContext>,
                event_tx: Option<tokio::sync::mpsc::Sender<$crate::agent::AgentEvent>>,
            ) -> Result<String, $crate::error::AgentError> {
                self.$inner_field
                    .invoke_with_hooks(message, context, event_tx, self.get_hooks())
                    .await
            }

            async fn invoke_stream(
                &self,
                message: $crate::types::Message,
                context: std::sync::Arc<$crate::agent::ExecutorContext>,
                event_tx: tokio::sync::mpsc::Sender<$crate::agent::AgentEvent>,
            ) -> Result<(), $crate::error::AgentError> {
                self.$inner_field
                    .invoke_stream_with_hooks(message, context, event_tx, self.get_hooks())
                    .await
            }
        }
    };
}
