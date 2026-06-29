//! Tests for new JSON parser (JSONL)

use super::super::ToolCallParser;
use super::super::json::JsonParser;
use super::TestData;

#[test]
fn test_jsonl_valid_parsing() {
    let parser = JsonParser::new(TestData::get_builtin_tool_names());
    let result = parser.parse(TestData::jsonl_valid());

    assert!(result.is_ok(), "Valid JSONL should parse successfully");

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 2, "Should parse 2 tool calls");

    // Check first tool call
    assert_eq!(tool_calls[0].tool_name, "search");
    let args = &tool_calls[0].input;
    assert_eq!(args["query"], "test query");
    assert_eq!(args["limit"], 5);

    // Check second tool call
    assert_eq!(tool_calls[1].tool_name, "final");
    let args = &tool_calls[1].input;
    assert_eq!(args["message"], "All done!");
}

#[test]
fn test_jsonl_complex_parsing() {
    let parser = JsonParser::new(TestData::get_builtin_tool_names());
    let result = parser.parse(TestData::jsonl_complex());

    assert!(result.is_ok(), "Complex JSONL should parse successfully");

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 2, "Should parse 2 tool calls");

    // Check first tool call (create_file)
    assert_eq!(tool_calls[0].tool_name, "create_file");
    let args = &tool_calls[0].input;
    assert_eq!(args["path"], "/tmp/test.txt");
    assert_eq!(args["content"], "Hello world!");
    assert_eq!(args["permissions"], 644);

    // Check second tool call (database_query)
    assert_eq!(tool_calls[1].tool_name, "database_query");
    let args = &tool_calls[1].input;
    assert_eq!(args["table"], "users");
    assert_eq!(args["limit"], 10);
    // Check nested object in filters
    let filters = &args["filters"];
    assert_eq!(filters["active"], true);
    assert_eq!(filters["role"], "admin");
}

#[test]
fn test_jsonl_invalid_parsing() {
    let parser = JsonParser::new(TestData::get_builtin_tool_names());
    let result = parser.parse(TestData::jsonl_invalid());

    assert!(result.is_err(), "Invalid JSONL should fail to parse");

    // Should return JSON parsing error
    match result {
        Err(distri_types::AgentError::JsonParsingFailed(content, _msg)) => {
            assert!(content.contains("Line 2"));
            // Don't check specific error message as it can vary
        }
        _ => panic!("Should return JsonParsingFailed error"),
    }
}

#[test]
fn test_jsonl_no_code_block() {
    let parser = JsonParser::new(TestData::get_builtin_tool_names());
    let jsonl_without_block = r#"{"name":"test","arguments":{"key":"value"}}"#;

    let result = parser.parse(jsonl_without_block);
    assert!(
        result.is_ok(),
        "JSONL without code block should still parse"
    );

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].tool_name, "test");
}

#[test]
fn test_jsonl_empty_lines() {
    let parser = JsonParser::new(TestData::get_builtin_tool_names());
    let jsonl_with_empty_lines = r#"```tool_calls
{"name":"first","arguments":{"param":"value1"}}

{"name":"second","arguments":{"param":"value2"}}
```"#;

    let result = parser.parse(jsonl_with_empty_lines);
    assert!(result.is_ok(), "JSONL with empty lines should parse");

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 2, "Should skip empty lines");
    assert_eq!(tool_calls[0].tool_name, "first");
    assert_eq!(tool_calls[1].tool_name, "second");
}

#[test]
fn test_jsonl_single_line() {
    let parser = JsonParser::new(TestData::get_builtin_tool_names());
    let single_line = r#"```tool_calls
{"name":"single","arguments":{"data":"test"}}
```"#;

    let result = parser.parse(single_line);
    assert!(result.is_ok(), "Single line JSONL should parse");

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].tool_name, "single");
    assert_eq!(tool_calls[0].input["data"], "test");
}

#[test]
fn test_jsonl_empty_arguments() {
    let parser = JsonParser::new(TestData::get_builtin_tool_names());
    let empty_args = r#"```tool_calls
{"name":"no_args","arguments":{}}
```"#;

    let result = parser.parse(empty_args);
    assert!(result.is_ok(), "JSONL with empty arguments should parse");

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].tool_name, "no_args");
    assert!(tool_calls[0].input.is_object());
    assert_eq!(tool_calls[0].input.as_object().unwrap().len(), 0);
}

#[test]
fn test_jsonl_format_name() {
    let parser = JsonParser::new(TestData::get_builtin_tool_names());
    assert_eq!(parser.format_name(), "JSON (JSONL)");
}

#[test]
fn test_jsonl_example_usage() {
    let parser = JsonParser::new(TestData::get_builtin_tool_names());
    let example = parser.example_usage();
    assert!(example.contains("```tool_calls"));
    assert!(example.contains(r#""name":"#));
    assert!(example.contains(r#""arguments":"#));
}
