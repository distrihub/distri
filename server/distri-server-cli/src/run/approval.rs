// if let Some(tool_call) = self.tool_calls.get_mut(&tool_call_id) {
//                     if tool_call.tool_call_name == APPROVAL_REQUEST_TOOL_NAME {
//                         if let Some(args) = tool_call.tool_call_args.as_ref() {
//                             // Parse approval request arguments
//                             if let Ok(approval_data) =
//                                 serde_json::from_str::<serde_json::Value>(args)
//                             {
//                                 let tool_calls = approval_data
//                                     .get("tool_calls")
//                                     .and_then(|v| v.as_array())
//                                     .map(|arr| {
//                                         arr.iter()
//                                             .filter_map(|tc| {
//                                                 serde_json::from_value::<ToolCall>(tc.clone()).ok()
//                                             })
//                                             .collect::<Vec<_>>()
//                                     })
//                                     .unwrap_or_default();

//                                 let reason = approval_data
//                                     .get("reason")
//                                     .and_then(|v| v.as_str())
//                                     .map(|s| s.to_string());

//                                 // Read user input for approval
//                                 let mut input = String::new();
//                                 if let Err(e) = io::stdin().read_line(&mut input) {
//                                     eprintln!("Error reading input: {}", e);
//                                     return Ok(EventPrinterResult::Exit);
//                                 }

//                                 let approved = input.trim().to_lowercase() == "y";

//                                 if approved {
//                                     println!(
//                                         "{}✅ Operation approved by user.{}",
//                                         COLOR_BRIGHT_GREEN, COLOR_RESET
//                                     );
//                                 } else {
//                                     println!(
//                                         "{}❌ Operation denied by user.{}",
//                                         COLOR_RED, COLOR_RESET
//                                     );
//                                 }

//                                 // Return the approval result to be handled by the caller
//                                 return Ok(EventPrinterResult::ApprovalRequired {
//                                     tool_call_id: tool_call_id.clone(),
//                                     tool_calls,
//                                     reason,
//                                 });
//                             }
//                         }
//                     } else if tool_call.tool_call_name == "execute_code" {
//                         if let Some(args) = tool_call.tool_call_args.as_ref() {
//                             if !parse_execute_code(args) {
//                                 // Fallback if parsing fails
//                                 println!("{}🔧 Tool arguments: {}", COLOR_BRIGHT_YELLOW, args);
//                             }
//                         }
//                     } else {
//                         // For other tools, just add a newline if verbose
//                         if self.verbose {
//                             println!();
//                         }
//                     }
//                 }
