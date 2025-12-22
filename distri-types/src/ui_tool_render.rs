use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

use crate::{Part, ToolCall, ToolResponse};

/// UI messages that can be generated for tool execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolUiMessage {
    pub message_type: ToolUiMessageType,
    pub parts: Vec<Part>,
}

/// Different types of UI messages for tool execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolUiMessageType {
    /// Message shown when tool starts executing
    ToolStart,
    /// Message shown when tool completes successfully
    ToolEnd,
    /// Message shown when tool fails
    ToolError,
    /// Progress message during tool execution (for long-running tools)
    ToolProgress,
}

/// Context for rendering UI messages
#[derive(Debug, Clone)]
pub struct ToolUiContext {
    pub tool_call: ToolCall,
    pub tool_response: Option<ToolResponse>,
    pub error: Option<String>,
    pub progress_info: Option<Value>,
}

/// Trait for rendering UI messages for tools
pub trait UiToolRender: Send + Sync + std::fmt::Debug {
    /// Get the tool name this renderer handles
    fn get_tool_name(&self) -> String;

    /// Generate tool start message
    fn render_tool_start(&self, context: &ToolUiContext) -> Result<ToolUiMessage>;

    /// Generate tool end message
    fn render_tool_end(&self, context: &ToolUiContext) -> Result<ToolUiMessage>;

    /// Generate tool error message
    fn render_tool_error(&self, context: &ToolUiContext) -> Result<ToolUiMessage>;

    /// Generate tool progress message (optional, for long-running tools)
    fn render_tool_progress(&self, _context: &ToolUiContext) -> Result<Option<ToolUiMessage>> {
        Ok(None) // Default: no progress messages
    }

    /// Check if this tool supports progress messages
    fn supports_progress(&self) -> bool {
        false // Default: no progress support
    }
}

/// Registry for tool UI renderers
#[derive(Debug, Default)]
pub struct ToolUiRenderRegistry {
    renderers: HashMap<String, Arc<dyn UiToolRender>>,
}

impl ToolUiRenderRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        Self {
            renderers: HashMap::new(),
        }
    }

    /// Register a UI renderer for a specific tool
    pub fn register(&mut self, tool_name: String, renderer: Arc<dyn UiToolRender>) {
        tracing::debug!("Registering UI renderer for tool: {}", tool_name);
        self.renderers.insert(tool_name, renderer);
    }

    /// Get a UI renderer for a specific tool
    pub fn get_renderer(&self, tool_name: &str) -> Option<&Arc<dyn UiToolRender>> {
        self.renderers.get(tool_name)
    }

    /// Render tool start message
    pub fn render_tool_start(&self, tool_call: &ToolCall) -> Result<ToolUiMessage> {
        let context = ToolUiContext {
            tool_call: tool_call.clone(),
            tool_response: None,
            error: None,
            progress_info: None,
        };

        if let Some(renderer) = self.get_renderer(&tool_call.tool_name) {
            renderer.render_tool_start(&context)
        } else {
            // Default rendering
            DefaultToolRenderer.render_tool_start(&context)
        }
    }

    /// Render tool end message
    pub fn render_tool_end(
        &self,
        tool_call: &ToolCall,
        tool_response: &ToolResponse,
    ) -> Result<ToolUiMessage> {
        let context = ToolUiContext {
            tool_call: tool_call.clone(),
            tool_response: Some(tool_response.clone()),
            error: None,
            progress_info: None,
        };

        if let Some(renderer) = self.get_renderer(&tool_call.tool_name) {
            renderer.render_tool_end(&context)
        } else {
            // Default rendering
            DefaultToolRenderer.render_tool_end(&context)
        }
    }

    /// Render tool error message
    pub fn render_tool_error(
        &self,
        tool_call: &ToolCall,
        error: &anyhow::Error,
    ) -> Result<ToolUiMessage> {
        let context = ToolUiContext {
            tool_call: tool_call.clone(),
            tool_response: None,
            error: Some(error.to_string()),
            progress_info: None,
        };

        if let Some(renderer) = self.get_renderer(&tool_call.tool_name) {
            renderer.render_tool_error(&context)
        } else {
            // Default rendering
            DefaultToolRenderer.render_tool_error(&context)
        }
    }

    /// Render tool progress message (if supported)
    pub fn render_tool_progress(
        &self,
        tool_call: &ToolCall,
        progress_info: Value,
    ) -> Result<Option<ToolUiMessage>> {
        let context = ToolUiContext {
            tool_call: tool_call.clone(),
            tool_response: None,
            error: None,
            progress_info: Some(progress_info),
        };

        if let Some(renderer) = self.get_renderer(&tool_call.tool_name) {
            if renderer.supports_progress() {
                return renderer.render_tool_progress(&context);
            }
        }

        Ok(None) // No progress rendering available
    }

    /// List all registered tool names
    pub fn list_registered_tools(&self) -> Vec<String> {
        self.renderers.keys().cloned().collect()
    }
}

/// Default tool renderer for tools without custom rendering
#[derive(Debug)]
pub struct DefaultToolRenderer;

impl UiToolRender for DefaultToolRenderer {
    fn get_tool_name(&self) -> String {
        "default".to_string()
    }

    fn render_tool_start(&self, context: &ToolUiContext) -> Result<ToolUiMessage> {
        let formatted_input =
            if context.tool_call.input.is_object() || context.tool_call.input.is_array() {
                serde_json::to_string_pretty(&context.tool_call.input)?
            } else {
                context.tool_call.input.to_string()
            };

        let message = format!(
            "🔧 **{}**\n\n```json\n{}\n```",
            context.tool_call.tool_name, formatted_input
        );

        Ok(ToolUiMessage {
            message_type: ToolUiMessageType::ToolStart,
            parts: vec![Part::Text(message)],
        })
    }

    fn render_tool_end(&self, context: &ToolUiContext) -> Result<ToolUiMessage> {
        let tool_response = context
            .tool_response
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Tool response required for tool_end message"))?;

        let message = format!("✅ **{}** completed", context.tool_call.tool_name);

        let mut parts = vec![Part::Text(message)];

        // Add tool response parts
        parts.extend(tool_response.parts.clone());

        Ok(ToolUiMessage {
            message_type: ToolUiMessageType::ToolEnd,
            parts,
        })
    }

    fn render_tool_error(&self, context: &ToolUiContext) -> Result<ToolUiMessage> {
        let error_msg = context
            .error
            .as_ref()
            .cloned()
            .unwrap_or_else(|| "Unknown error".to_string());

        let message = format!(
            "❌ **{}** failed\n\n```\n{}\n```",
            context.tool_call.tool_name, error_msg
        );

        Ok(ToolUiMessage {
            message_type: ToolUiMessageType::ToolError,
            parts: vec![Part::Text(message)],
        })
    }
}

/// Helper function to create a global registry instance
pub fn create_default_registry() -> ToolUiRenderRegistry {
    let mut registry = ToolUiRenderRegistry::new();

    // Register built-in tool renderers
    crate::ui_tool_renderers::register_common_renderers(&mut registry);

    registry
}
