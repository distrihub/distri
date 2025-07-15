#[cfg(test)]
mod tests {
    use crate::{
        agent::{AgentExecutorBuilder, ExecutorContext, ToolParserAgent},
        stores::noop::NoopSessionStore,
        tool_formatter::ToolCallFormat,
        tools::LlmToolsRegistry,
        types::{AgentDefinition, ModelSettings},
        SessionStore,
    };
    use std::sync::Arc;

    #[test]
    fn test_parse_xml_tool_calls() {
        let agent = create_test_agent();

        // Test XML tool call parsing
        let content = r#"
        I need to search for something.
        <tool_call name="search" args='{"query": "test query", "limit": 10}'>
        This should be parsed as a tool call.
        "#;

        let result = agent.parse_tool_calls(content);
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
        let agent = create_test_agent();

        // Test JSON tool call parsing
        let content = r#"
        I need to call a tool.
        {"tool": "calculator", "args": {"operation": "add", "a": 5, "b": 3}}
        This should be parsed as a tool call.
        "#;

        let result = agent.parse_tool_calls(content);
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
        let agent = create_test_agent();

        // Test content with no tool calls
        let content = "This is just regular text with no tool calls.";

        let result = agent.parse_tool_calls(content);
        assert!(result.is_ok());

        let tool_calls = result.unwrap();
        assert_eq!(tool_calls.len(), 0);
    }

    fn create_test_agent() -> ToolParserAgent {
        let definition = AgentDefinition {
            name: "test_agent".to_string(),
            description: "Test agent".to_string(),
            version: None,
            agent_type: None,
            system_prompt: None,
            mcp_servers: vec![],
            model_settings: ModelSettings::default(),
            history_size: None,
            plan: None,
            icon_url: None,
            max_iterations: None,
            skills: vec![],
            sub_agents: vec![],
        };

        let tools_registry = Arc::new(LlmToolsRegistry::default());
        let coordinator = Arc::new(AgentExecutorBuilder::default().build().unwrap());
        let context = Arc::new(ExecutorContext::default());
        let session_store =
            Arc::new(Box::new(NoopSessionStore::default()) as Box<dyn SessionStore>);

        ToolParserAgent::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
            ToolCallFormat::Current,
        )
    }
}
