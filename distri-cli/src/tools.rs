use std::io::{self, Write};

use distri::ExternalToolRegistry;

use crate::{COLOR_BRIGHT_GREEN, COLOR_BRIGHT_MAGENTA, COLOR_BRIGHT_YELLOW, COLOR_RESET};

pub fn register_approval_handler(registry: &ExternalToolRegistry) {
    registry.register("*", "approval_request", |call, _event| async move {
        println!(
            "{}Calling tool:{} {}",
            COLOR_BRIGHT_MAGENTA, COLOR_RESET, call.tool_name
        );
        println!("{}Approval required{}", COLOR_BRIGHT_YELLOW, COLOR_RESET);
        print!(
            "{}Do you approve this operation? (y/n): {}",
            COLOR_BRIGHT_YELLOW, COLOR_RESET
        );
        io::stdout().flush().ok();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return Err(anyhow::anyhow!("Failed to read approval input"));
        }

        let approved = input.trim().eq_ignore_ascii_case("y");
        if approved {
            println!(
                "{}Operation approved by user.{}",
                COLOR_BRIGHT_GREEN, COLOR_RESET
            );
        } else {
            println!("Operation rejected by user.");
        }

        let tool_calls = call.input.clone();
        let approval_result = serde_json::json!({
            "approved": approved,
            "reason": if approved { "Approved by user" } else { "Rejected by user" },
            "tool_calls": tool_calls,
        });

        Ok(distri_types::ToolResponse::direct(
            call.tool_call_id.clone(),
            call.tool_name.clone(),
            approval_result,
        ))
    });
}
