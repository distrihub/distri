use anyhow::Result;
use std::sync::Arc;

use crate::{Part, ToolUiContext, ToolUiMessage, ToolUiMessageType, UiToolRender};

/// UI renderer for search/grep tools
#[derive(Debug)]
pub struct SearchToolRenderer;

impl UiToolRender for SearchToolRenderer {
    fn get_tool_name(&self) -> String {
        "search".to_string()
    }

    fn render_tool_start(&self, context: &ToolUiContext) -> Result<ToolUiMessage> {
        let query = context
            .tool_call
            .input
            .get("query")
            .and_then(|q| q.as_str())
            .unwrap_or("unknown");

        let message = format!("🔍 **Searching for:** `{}`", query);

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

        let mut parts = vec![Part::Text("✅ **Search completed**".to_string())];
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
            .unwrap_or_else(|| "Search failed".to_string());

        let message = format!("❌ **Search failed**\n\n```\n{}\n```", error_msg);

        Ok(ToolUiMessage {
            message_type: ToolUiMessageType::ToolError,
            parts: vec![Part::Text(message)],
        })
    }
}

/// UI renderer for file operations (read, write, etc.)
#[derive(Debug)]
pub struct FileToolRenderer;

impl UiToolRender for FileToolRenderer {
    fn get_tool_name(&self) -> String {
        "file_read".to_string() // Can be used for multiple file tools
    }

    fn render_tool_start(&self, context: &ToolUiContext) -> Result<ToolUiMessage> {
        let tool_name = &context.tool_call.tool_name;
        let path = context
            .tool_call
            .input
            .get("path")
            .and_then(|p| p.as_str())
            .unwrap_or("unknown");

        let action = match tool_name.as_str() {
            name if name.contains("read") => "📖 Reading",
            name if name.contains("write") => "✏️ Writing",
            name if name.contains("delete") => "🗑️ Deleting",
            name if name.contains("copy") => "📋 Copying",
            name if name.contains("move") => "🔄 Moving",
            _ => "📁 Processing",
        };

        let message = format!("{} **{}**", action, path);

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

        let tool_name = &context.tool_call.tool_name;
        let action = match tool_name.as_str() {
            name if name.contains("read") => "Read",
            name if name.contains("write") => "Wrote",
            name if name.contains("delete") => "Deleted",
            name if name.contains("copy") => "Copied",
            name if name.contains("move") => "Moved",
            _ => "Processed",
        };

        let mut parts = vec![Part::Text(format!("✅ **{} completed**", action))];
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
            .unwrap_or_else(|| "File operation failed".to_string());

        let message = format!("❌ **File operation failed**\n\n```\n{}\n```", error_msg);

        Ok(ToolUiMessage {
            message_type: ToolUiMessageType::ToolError,
            parts: vec![Part::Text(message)],
        })
    }
}

/// UI renderer for code execution tools
#[derive(Debug)]
pub struct CodeExecutionToolRenderer;

impl UiToolRender for CodeExecutionToolRenderer {
    fn get_tool_name(&self) -> String {
        "distri_execute_code".to_string()
    }

    fn render_tool_start(&self, context: &ToolUiContext) -> Result<ToolUiMessage> {
        let code = context
            .tool_call
            .input
            .get("code")
            .and_then(|c| c.as_str())
            .unwrap_or("");

        let language = context
            .tool_call
            .input
            .get("language")
            .and_then(|l| l.as_str())
            .unwrap_or("javascript");

        let preview = if code.len() > 100 {
            format!("{}...", &code[..100])
        } else {
            code.to_string()
        };

        let message = format!(
            "⚡ **Executing {} code**\n\n```{}\n{}\n```",
            language, language, preview
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

        let mut parts = vec![Part::Text("✅ **Code execution completed**".to_string())];
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
            .unwrap_or_else(|| "Code execution failed".to_string());

        let message = format!("❌ **Code execution failed**\n\n```\n{}\n```", error_msg);

        Ok(ToolUiMessage {
            message_type: ToolUiMessageType::ToolError,
            parts: vec![Part::Text(message)],
        })
    }

    fn supports_progress(&self) -> bool {
        true
    }

    fn render_tool_progress(&self, context: &ToolUiContext) -> Result<Option<ToolUiMessage>> {
        if let Some(progress_info) = &context.progress_info {
            if let Some(status) = progress_info.get("status").and_then(|s| s.as_str()) {
                let message = format!("⏳ **Code execution:** {}", status);

                return Ok(Some(ToolUiMessage {
                    message_type: ToolUiMessageType::ToolProgress,
                    parts: vec![Part::Text(message)],
                }));
            }
        }

        Ok(None)
    }
}

/// Helper function to register common tool renderers
pub fn register_common_renderers(registry: &mut crate::ToolUiRenderRegistry) {
    registry.register("search".to_string(), Arc::new(SearchToolRenderer));
    registry.register("file_read".to_string(), Arc::new(FileToolRenderer));
    registry.register("file_write".to_string(), Arc::new(FileToolRenderer));
    registry.register("file_delete".to_string(), Arc::new(FileToolRenderer));
    registry.register("file_copy".to_string(), Arc::new(FileToolRenderer));
    registry.register("file_move".to_string(), Arc::new(FileToolRenderer));
    registry.register(
        "distri_execute_code".to_string(),
        Arc::new(CodeExecutionToolRenderer),
    );
}
