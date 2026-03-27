use std::io::{self, Write};

use distri::{Distri, ExternalToolRegistry};

use crate::{COLOR_BRIGHT_GREEN, COLOR_BRIGHT_MAGENTA, COLOR_BRIGHT_YELLOW, COLOR_RESET};

/// Register an api_request handler on the external tool registry.
/// When the agent calls api_request, the CLI executes the HTTP request
/// using the Distri client and returns the result.
pub fn register_api_request_handler(registry: &ExternalToolRegistry, client: Distri) {
    let client = std::sync::Arc::new(client);
    registry.register("*", "api_request", move |call, _event| {
        let client = client.clone();
        async move {
            let result = distri::execute_api_request(&client, &call.input).await;
            Ok(distri_types::ToolResponse {
                tool_call_id: call.tool_call_id,
                tool_name: call.tool_name,
                parts: vec![distri_types::Part::Data(result)],
                parts_metadata: None,
            })
        }
    });
}

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
