#[cfg(test)]
mod tests {
    use crate::{agent::hooks::ToolParsingHooks, tool_formatter::ToolCallFormat};

    #[test]
    fn test_parse_xml_tool_calls() {
        let hook = create_tool_parsing_hook();

        // Test XML tool call parsing
        let content = r#"
        I need to search for something.
        <tool_call name="search" args='{"query": "test query", "limit": 10}'>
        This should be parsed as a tool call.
        "#;

        let result = hook.parse_tool_calls(content);
        assert!(result.is_ok());

        let tool_calls = result.unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].tool_name, "search");
        assert_eq!(
            tool_calls[0].input,
            r#"{"query": "test query", "limit": 10}"#
        );
    }

    #[test]
    fn test_parse_json_tool_calls() {
        let hook = create_tool_parsing_hook();

        // Test JSON tool call parsing
        let content = r#"
        I need to call a tool.
        {"tool": "calculator", "args": {"operation": "add", "a": 5, "b": 3}}
        This should be parsed as a tool call.
        "#;

        let result = hook.parse_tool_calls(content);
        assert!(result.is_ok());

        let tool_calls = result.unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].tool_name, "calculator");
        assert_eq!(
            tool_calls[0].input,
            r#"{"operation": "add", "a": 5, "b": 3}"#
        );
    }

    #[test]
    fn test_no_tool_calls() {
        let hook = create_tool_parsing_hook();

        // Test content with no tool calls
        let content = "This is just regular text with no tool calls.";

        let result = hook.parse_tool_calls(content);
        assert!(result.is_ok());

        let tool_calls = result.unwrap();
        assert_eq!(tool_calls.len(), 0);
    }

    fn create_tool_parsing_hook() -> ToolParsingHooks {
        ToolParsingHooks::new(ToolCallFormat::Xml, vec![])
    }
}
