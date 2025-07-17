use crate::tool_formatter::{ToolCallFormat, ToolCallWrapper};
use crate::types::ToolCall;

#[test]
fn test_current_format_parsing() {
    let content = r#"
    Here's my response with tool calls:
    
    <tool_calls>
    <tool_call name="search" args='{"query": "rust programming"}' />
    <tool_call name="scrape" args='{"url": "https://example.com"}' />
    </tool_calls>
    
    That's all I have to say.
    "#;

    let tool_calls = ToolCallWrapper::parse_from_xml(content, ToolCallFormat::Xml).unwrap();

    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].tool_name, "search");
    assert_eq!(tool_calls[0].input, r#"{"query": "rust programming"}"#);
    assert_eq!(tool_calls[1].tool_name, "scrape");
    assert_eq!(tool_calls[1].input, r#"{"url": "https://example.com"}"#);
}

#[test]
fn test_current_format_generation() {
    let tool_calls = vec![
        ToolCall {
            tool_call_id: "1".to_string(),
            tool_name: "search".to_string(),
            input: r#"{"query": "test"}"#.to_string(),
        },
        ToolCall {
            tool_call_id: "2".to_string(),
            tool_name: "scrape".to_string(),
            input: r#"{"url": "https://example.com"}"#.to_string(),
        },
    ];

    let wrapper = ToolCallWrapper {
        format: ToolCallFormat::Xml,
        tool_calls,
    };

    let xml = wrapper.to_xml(&ToolCallFormat::Xml);
    let expected = r#"<tool_calls>
<tool_call name="search" args='{"query": "test"}' />
<tool_call name="scrape" args='{"url": "https://example.com"}' />
</tool_calls>"#;

    assert_eq!(xml.trim(), expected);
}

#[test]
fn test_fallback_parsing_without_wrapper() {
    let content = r#"
    Here's my response with tool calls:
    
    <tool_call name="search" args='{"query": "rust programming"}' />
    <tool_call name="scrape" args='{"url": "https://example.com"}' />
    
    That's all I have to say.
    "#;

    let tool_calls = ToolCallWrapper::parse_from_xml(content, ToolCallFormat::Xml).unwrap();

    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].tool_name, "search");
    assert_eq!(tool_calls[1].tool_name, "scrape");
}
