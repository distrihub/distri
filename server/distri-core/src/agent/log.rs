use std::sync::Arc;

use async_openai::types::chat::CreateChatCompletionRequest;
use comfy_table::{
    modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, Cell, Color, ContentArrangement, Table,
};
use tracing::info;

use crate::agent::ExecutorContext;

#[derive(Debug, Clone)]
pub struct ModelLogger {
    verbose: bool,
}

impl ModelLogger {
    pub fn new(verbose: Option<bool>) -> Self {
        let verbose = verbose.unwrap_or(std::env::var("DISTRI_LOG_MESSAGES").is_ok());
        Self { verbose }
    }

    pub fn log_openai_messages(&self, request: &CreateChatCompletionRequest) {
        if !self.verbose {
            return;
        }

        let timestamp = chrono::Utc::now().timestamp_millis().to_string();
        std::fs::create_dir_all(".distri/requests").unwrap();
        std::fs::write(
            format!(".distri/requests/{timestamp}.json", timestamp = timestamp),
            serde_json::to_string(&request).unwrap(),
        )
        .unwrap();
    }

    /// Truncate content while preserving formatting and code blocks
    fn truncate_preserving_formatting(&self, text: &str, max_len: usize) -> String {
        if text.len() <= max_len {
            return text.to_string();
        }

        // Try to find a good truncation point
        let mut truncated = text[..max_len].to_string();

        // If we're in the middle of a code block, try to find the end
        if truncated.contains("```") {
            let code_block_count = truncated.matches("```").count();
            if code_block_count % 2 == 1 {
                // We're in the middle of a code block, try to find the closing ```
                if let Some(end_pos) = text[max_len..].find("```") {
                    let new_end = max_len + end_pos + 3;
                    if new_end <= text.len() {
                        truncated = text[..new_end].to_string();
                    }
                }
            }
        }

        // If we're in the middle of a line, try to find the end of the line
        if !truncated.ends_with('\n') {
            if let Some(newline_pos) = text[max_len..].find('\n') {
                let new_end = max_len + newline_pos + 1;
                if new_end <= text.len() {
                    truncated = text[..new_end].to_string();
                }
            }
        }

        format!(
            "{}\n\n[Content truncated - {} chars total]",
            truncated,
            text.len()
        )
    }

    /// Log messages in a more readable format (alternative to table format)

    pub async fn log_scratchpad(&self, context: &Arc<ExecutorContext>) {
        if !self.verbose {
            return;
        }

        let agent_id = context.agent_id.clone();
        let mut scratchpad_str = String::new();
        scratchpad_str.push_str(&format!("Agent: {}\n", agent_id));
        scratchpad_str.push_str(&format!("thread_id: {}\n", context.thread_id));
        scratchpad_str.push_str(&format!("task_id: {}\n", context.task_id));
        scratchpad_str.push_str(&format!("run_id: {}\n", context.run_id));
        scratchpad_str.push_str(&format!(
            "┌──────────────────────────────────────────────────────────────────────────────────────────┐\n"
        ));
        let scratchpad = context.format_agent_scratchpad(Some(10)).await;
        if let Ok(s) = scratchpad {
            scratchpad_str.push_str(&s);
        }
        scratchpad_str.push_str(&format!(
            "└──────────────────────────────────────────────────────────────────────────────────────────┘\n"
        ));
        tracing::info!("📋 Scratchpad:\n{}", scratchpad_str);
    }

    fn truncate_for_display(&self, text: &str, max_len: usize) -> String {
        if text.len() <= max_len {
            text.to_string()
        } else {
            // Use the improved truncation that preserves formatting
            self.truncate_preserving_formatting(text, max_len)
        }
    }

    pub fn log_model_execution(
        &self,
        agent_name: &str,
        model_name: &str,
        messages_count: usize,
        settings: Option<&str>,
        token_usage: Option<u32>,
    ) {
        if !self.verbose {
            return;
        }

        let mut table = Table::new()
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .to_owned();
        table.set_header(vec![
            "Agent",
            "Model",
            "Messages",
            "Settings",
            "Token Usage",
        ]);

        let settings_str = settings.unwrap_or("None");
        let token_str = token_usage.map_or("None".to_string(), |t| t.to_string());

        table.add_row(vec![
            agent_name,
            model_name,
            &messages_count.to_string(),
            settings_str,
            &token_str,
        ]);

        info!("\n{}", table);
    }

    /// Log tool execution details
    pub fn log_tool_execution(&self, tool_name: &str, input: &str, output: &serde_json::Value) {
        if !self.verbose {
            return;
        }

        let mut table = Table::new()
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_width(120)
            .to_owned();

        table.set_header(vec!["Tool Execution Debug", "Details"]);

        let input_preview = self.truncate_for_display(input, 300);
        let output_str = serde_json::to_string_pretty(output).unwrap_or_default();
        let output_preview = self.truncate_for_display(&output_str, 300);

        table.add_row(vec![
            Cell::new("Tool Name").fg(Color::Blue),
            Cell::new(tool_name),
        ]);
        table.add_row(vec![
            Cell::new("Input").fg(Color::Yellow),
            Cell::new(&input_preview),
        ]);
        table.add_row(vec![
            Cell::new("Output").fg(Color::Green),
            Cell::new(&output_preview),
        ]);

        tracing::info!("\n🔧 TOOL EXECUTION:\n{}", table);
    }

    /// Log parsing errors for debugging
    pub fn log_parsing_error(&self, parser_type: &str, input: &str, error: &str) {
        if !self.verbose {
            return;
        }

        let mut table = Table::new()
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_width(120)
            .to_owned();

        table.set_header(vec![Cell::new("Parsing Error Debug").fg(Color::Red)]);

        let input_preview = self.truncate_for_display(input, 400);

        table.add_row(vec![Cell::new(&format!(
            "Parser: {}\n\nInput:\n{}\n\nError:\n{}",
            parser_type, input_preview, error
        ))
        .fg(Color::Red)]);

        tracing::error!("\n❌ PARSING ERROR:\n{}", table);
    }
}
