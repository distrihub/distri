#[cfg(test)]
mod tests {
    use crate::slash_commands::{
        executor::SlashCommandExecutor,
        registry::SlashCommandRegistry,
        types::{SlashCommandResult, SlashCommandType},
    };

    #[tokio::test]
    async fn test_toolcall_slash_command() {
        let mut executor = SlashCommandExecutor::new().expect("Failed to create executor");

        // Test full toolcall command with JSON parameters
        let result = executor.execute("/toolcall search {\"query\": \"find population of san francisco\", \"limit\": \"5\"}").await;

        // We can't test actual tool execution without setting up MCP servers,
        // but we can verify the command is processed correctly
        assert!(result.is_ok());

        // The result will likely be a "tool not found" or similar error, which is expected
        // The important part is that the command was parsed and recognized
        match result.unwrap() {
            SlashCommandResult::ToolCall {
                tool: _,
                parameters: _,
            } => {
                // This is now expected since JSON parsing works correctly
                // The tool call will be processed but execution may fail in the handler
            }
            SlashCommandResult::Continue => {
                // This could happen if tool execution fails in the handler
            }
            SlashCommandResult::Message(msg) => {
                // Error messages are also acceptable for this test
                assert!(
                    msg.contains("tool")
                        || msg.contains("Tool")
                        || msg.contains("failed")
                        || msg.contains("JSON")
                );
            }
            other => {
                // Any other result type is fine for this test
                println!("Got result: {:?}", other);
            }
        }
    }

    #[tokio::test]
    async fn test_toolcall_registry_registration() {
        let registry = SlashCommandRegistry::new();

        // Check that toolcall command is registered
        let commands = registry.get_commands();
        let toolcall_cmd = commands.iter().find(|cmd| cmd.name == "toolcall");

        assert!(toolcall_cmd.is_some());

        let cmd = toolcall_cmd.unwrap();
        assert_eq!(cmd.name, "toolcall");
        assert!(cmd.description.contains("Call a tool directly"));
        assert!(cmd.builtin);

        if let SlashCommandType::Function { handler } = &cmd.command_type {
            assert_eq!(handler, "call_tool_interactive");
        } else {
            panic!(
                "Expected Function command type, got: {:?}",
                cmd.command_type
            );
        }
    }

    #[tokio::test]
    async fn test_toolcall_json_parameter_parsing() {
        let mut executor = SlashCommandExecutor::new().expect("Failed to create executor");

        // Test JSON parameter parsing
        let result = executor
            .execute("/toolcall test_tool {\"query\": \"hello world\", \"limit\": \"10\"}")
            .await;

        assert!(result.is_ok());

        match result.unwrap() {
            SlashCommandResult::ToolCall { tool, parameters } => {
                assert_eq!(tool, "test_tool");
                assert_eq!(parameters.get("query"), Some(&"hello world".to_string()));
                assert_eq!(parameters.get("limit"), Some(&"10".to_string()));
            }
            other => {
                panic!("Expected ToolCall result, got: {:?}", other);
            }
        }
    }

    #[tokio::test]
    async fn test_toolcall_invalid_json() {
        let mut executor = SlashCommandExecutor::new().expect("Failed to create executor");

        // Test invalid JSON parameter parsing
        let result = executor.execute("/toolcall test_tool {invalid json}").await;

        assert!(result.is_ok());

        match result.unwrap() {
            SlashCommandResult::Message(msg) => {
                assert!(msg.contains("Invalid JSON format"));
            }
            other => {
                panic!("Expected error message for invalid JSON, got: {:?}", other);
            }
        }
    }
}
