//! Tests for new XML parser

use super::super::ToolCallParser;
use super::super::xml::XmlParser;
use super::TestData;

#[test]
fn test_xml_valid_parsing() {
    let parser = XmlParser::new(TestData::get_builtin_tool_names());
    let result = parser.parse(TestData::xml_valid());

    assert!(result.is_ok(), "Valid XML should parse successfully");

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
fn test_xml_ministers_parsing() {
    let parser = XmlParser::new(vec!["search".to_string()]);
    let result = parser.parse(r#"<search>List of current ministers of Singapore</search>"#);
    assert!(result.is_ok(), "Ministers XML should parse successfully");

    let tool_calls = result.unwrap();
    println!("{:?}", tool_calls);
    assert_eq!(tool_calls.len(), 1, "Should parse 1 tool call");
}
#[test]
fn test_xml_search_streaming() {
    use super::super::ToolCallParser;

    let mut parser = XmlParser::new(vec!["search".to_string()]);
    let xml_chunks = vec![
        "<search>\n  <query>top 10 popular",
        " food items</query>\n  <limit>5</limit>\n</search>",
    ];

    let mut all_tool_calls = vec![];

    // Process each chunk
    for chunk in xml_chunks {
        let result = parser.process_chunk(chunk);
        assert!(result.is_ok(), "Streaming XML should process successfully");

        let stream_result = result.unwrap();
        let tool_calls_count = stream_result.new_tool_calls.len();
        all_tool_calls.extend(stream_result.new_tool_calls);
        println!("After chunk '{}': {} tool calls", chunk, tool_calls_count);
    }

    // Finalize
    let final_result = parser.finalize();
    assert!(final_result.is_ok(), "Finalize should succeed");
    all_tool_calls.extend(final_result.unwrap());

    println!("Final streaming result: {:?}", all_tool_calls);
    assert_eq!(
        all_tool_calls.len(),
        1,
        "Should parse 1 tool call via streaming"
    );
    assert_eq!(all_tool_calls[0].tool_name, "search");

    let input = &all_tool_calls[0].input;
    println!("Streaming input: {:?}", input);

    // Should be structured parameters, not empty
    assert!(
        input.is_object(),
        "Search tool should receive structured parameters via streaming"
    );
    let params = input.as_object().unwrap();
    assert_eq!(params.get("query").unwrap(), "top 10 popular food items");
    assert_eq!(params.get("limit").unwrap(), 5);
}

#[test]
fn test_xml_complex_parsing() {
    let parser = XmlParser::new(vec![
        "create_file".to_string(),
        "database_query".to_string(),
    ]);
    let result = parser.parse(TestData::xml_complex());

    assert!(result.is_ok(), "Complex XML should parse successfully");

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
    // Check nested JSON in filters
    let filters = &args["filters"];
    assert_eq!(filters["active"], true);
    assert_eq!(filters["role"], "admin");
}

#[test]
fn test_xml_malformed_parsing() {
    // Test that malformed XML (unclosed tags) can still be parsed robustly
    let parser = XmlParser::new(TestData::get_builtin_tool_names());
    let result = parser.parse(TestData::xml_invalid());

    assert!(result.is_ok(), "Robust parser should handle malformed XML");

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1, "Should parse 1 tool call");

    // Check the parsed tool call
    assert_eq!(tool_calls[0].tool_name, "search");
    let args = &tool_calls[0].input;
    assert_eq!(args["query"], "test query");

    // The unclosed <limit> tag should still be parsed, taking content to end
    // Our robust parser converts "5" to a number
    assert_eq!(args["limit"], 5);
}

#[test]
fn test_xml_no_code_block() {
    let parser = XmlParser::new(TestData::get_builtin_tool_names());
    let xml_without_block = r#"<search>
  <query>test query</query>
</search>"#;

    let result = parser.parse(xml_without_block);
    assert!(result.is_ok(), "XML without code block should still parse");

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].tool_name, "search");
}

#[test]
fn test_xml_empty_tool() {
    let parser = XmlParser::new(TestData::get_builtin_tool_names());
    let empty_tool = r#"```tool_calls
<empty_tool>
</empty_tool>
```"#;

    let result = parser.parse(empty_tool);
    assert!(result.is_ok(), "Empty tool should parse");

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].tool_name, "empty_tool");

    // Empty tools now return empty string (consistent with string parameter expectation)
    assert!(tool_calls[0].input.is_string());
    assert_eq!(tool_calls[0].input, "");
}

#[test]
fn test_xml_boolean_and_number_parsing() {
    let parser = XmlParser::new(TestData::get_builtin_tool_names());
    let xml_types = r#"```tool_calls
<test_types>
  <enabled>true</enabled>
  <disabled>false</disabled>
  <count>42</count>
  <price>19.99</price>
  <name>test string</name>
</test_types>
```"#;

    let result = parser.parse(xml_types);
    assert!(result.is_ok(), "XML with different types should parse");

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1);

    let args = &tool_calls[0].input;
    assert_eq!(args["enabled"], true);
    assert_eq!(args["disabled"], false);
    assert_eq!(args["count"], 42);
    assert_eq!(args["price"], 19.99);
    assert_eq!(args["name"], "test string");
}

#[test]
fn test_xml_string_parameter() {
    // Test that tools with simple string content are parsed correctly
    let parser = XmlParser::new(vec!["call_search_agent".to_string()]);
    let xml_content = r#"<call_search_agent>Find the top 10 children's books with their titles and authors as a list of [Title, Author].</call_search_agent>"#;

    let result = parser.parse(xml_content);
    assert!(
        result.is_ok(),
        "String parameter XML should parse successfully"
    );

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1, "Should parse 1 tool call");
    assert_eq!(tool_calls[0].tool_name, "call_search_agent");

    // The content should be treated as a single string parameter
    let input = &tool_calls[0].input;

    if let Some(task) = input.get("task") {
        assert_eq!(
            task,
            "Find the top 10 children's books with their titles and authors as a list of [Title, Author]."
        );
    } else if input.is_string() {
        // Alternative: treat the whole content as a string
        assert_eq!(
            input,
            "Find the top 10 children's books with their titles and authors as a list of [Title, Author]."
        );
    } else {
        panic!(
            "Tool input should be either a string or have a 'task' parameter, got: {:?}",
            input
        );
    }
}

#[test]
fn test_xml_string_parameter_with_unencoded_characters() {
    // Test unencoded special characters (now allowed per updated instructions)
    let parser = XmlParser::new(vec!["call_search_agent".to_string()]);
    let xml_content = r#"<call_search_agent>Find the top 10 children's books with their titles and authors as a list of [Title, Author].</call_search_agent>"#;

    let result = parser.parse(xml_content);
    assert!(
        result.is_ok(),
        "String parameter XML with unencoded characters should parse successfully"
    );

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1, "Should parse 1 tool call");
    assert_eq!(tool_calls[0].tool_name, "call_search_agent");

    // The content should be treated as a string parameter
    let input = &tool_calls[0].input;

    let expected_text = "Find the top 10 children's books with their titles and authors as a list of [Title, Author].";

    if input.is_string() {
        assert_eq!(input, expected_text);
    } else {
        panic!("Tool input should be a string, got: {:?}", input);
    }
}

#[test]
fn test_xml_string_parameter_with_encoded_entities() {
    // Test that encoded entities are handled without decoding
    let parser = XmlParser::new(vec!["call_search_agent".to_string()]);
    let xml_content = r#"<call_search_agent>Find the top 10 children&apos;s books with their titles and authors as a list of [Title, Author].</call_search_agent>"#;

    let result = parser.parse(xml_content);
    assert!(result.is_ok(), "Parser should handle encoded entities");

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1, "Should parse 1 tool call");
    assert_eq!(tool_calls[0].tool_name, "call_search_agent");

    let input = &tool_calls[0].input;

    // Our parser doesn't decode &apos; so it remains as-is
    let expected_text_raw = "Find the top 10 children&apos;s books with their titles and authors as a list of [Title, Author].";

    assert!(input.is_string(), "Tool input should be a string");
    assert_eq!(input, expected_text_raw);
}

#[test]
fn test_xml_reproducing_empty_object_issue() {
    // Try to reproduce the exact issue where the parser returns {} instead of string content
    let parser = XmlParser::new(vec![]); // Empty valid tools list to mimic real scenario

    // Test both with and without entities
    let test_cases = vec![
        r#"<call_search_agent>Find the top 10 children's books with their titles and authors as a list of [Title, Author].</call_search_agent>"#,
        r#"<call_search_agent>Find the top 10 children&apos;s books with their titles and authors as a list of [Title, Author].</call_search_agent>"#,
    ];

    for (i, xml_content) in test_cases.iter().enumerate() {
        let result = parser.parse(xml_content);
        assert!(result.is_ok(), "Parser should handle case {}", i + 1);

        let tool_calls = result.unwrap();
        assert_eq!(
            tool_calls.len(),
            1,
            "Should parse 1 tool call for case {}",
            i + 1
        );
        assert_eq!(tool_calls[0].tool_name, "call_search_agent");

        let input = &tool_calls[0].input;

        // This should NOT be an empty object
        assert!(
            !input.is_object() || input.as_object().unwrap().len() > 0,
            "Input should not be an empty object for case {}",
            i + 1
        );

        // Should be a string
        if !input.is_string() {
            panic!("Expected string input for case {}, got: {:?}", i + 1, input);
        }
    }
}

#[test]
fn test_xml_final_tool_string_parameter() {
    // Test the final tool which should receive string input
    let parser = XmlParser::new(vec!["final".to_string()]);
    let xml_content = r#"<final>Added the value "Add" to cell A1 (row 0, col 0) in the first sheet using the appropriate setValue BlinkOp.</final>"#;

    let result = parser.parse(xml_content);
    assert!(result.is_ok(), "Final tool XML should parse successfully");

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1, "Should parse 1 tool call");
    assert_eq!(tool_calls[0].tool_name, "final");

    let input = &tool_calls[0].input;

    // Final tool should receive a string, not an object
    assert!(
        input.is_string(),
        "Final tool input should be a string, got: {:?}",
        input
    );
    assert_eq!(
        input,
        r#"Added the value "Add" to cell A1 (row 0, col 0) in the first sheet using the appropriate setValue BlinkOp."#
    );
}

#[test]
fn test_xml_final_tool_empty_object_scenario() {
    // Test scenarios that might cause final tool to receive empty object
    let parser = XmlParser::new(vec![]); // Empty valid tools list to mimic real scenario

    let test_cases = vec![
        r#"<final>Added the value "Add" to cell A1 (row 0, col 0) in the first sheet using the appropriate setValue BlinkOp.</final>"#,
        r#"<final>Task completed successfully</final>"#,
        r#"<final></final>"#, // Empty final
    ];

    for (i, xml_content) in test_cases.iter().enumerate() {
        let result = parser.parse(xml_content);
        assert!(
            result.is_ok(),
            "Final tool case {} should parse successfully",
            i + 1
        );

        let tool_calls = result.unwrap();
        assert_eq!(
            tool_calls.len(),
            1,
            "Should parse 1 tool call for case {}",
            i + 1
        );
        assert_eq!(tool_calls[0].tool_name, "final");

        let input = &tool_calls[0].input;

        // Should NOT be an empty object
        if input.is_object() && input.as_object().unwrap().is_empty() {
            panic!(
                "Case {}: Final tool received empty object instead of string content. XML: {}",
                i + 1,
                xml_content
            );
        }

        // Should be a string (or empty string for case 3)
        assert!(
            input.is_string(),
            "Case {}: Final tool input should be a string, got: {:?}",
            i + 1,
            input
        );

        match i {
            0 => assert_eq!(
                input,
                r#"Added the value "Add" to cell A1 (row 0, col 0) in the first sheet using the appropriate setValue BlinkOp."#
            ),
            1 => assert_eq!(input, "Task completed successfully"),
            2 => assert_eq!(input, ""), // Empty string
            _ => {}
        }
    }
}

#[test]
fn test_xml_final_exact_failing_case() {
    // Test the exact failing case from the error
    let parser = XmlParser::new(vec!["final".to_string()]);
    let xml_content = r#"<final>Added the value "Add" to cell A1 (row 0, col 0) in the first sheet using the appropriate setValue BlinkOp.</final>"#;

    let result = parser.parse(xml_content);

    assert!(result.is_ok(), "Final tool XML should parse successfully");

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1, "Should parse 1 tool call");
    assert_eq!(tool_calls[0].tool_name, "final");

    let input = &tool_calls[0].input;

    // This should NOT be an empty object
    if input.is_object() && input.as_object().unwrap().is_empty() {
        panic!("Final tool received empty object {{}} instead of string content!");
    }

    // Should be a string
    assert!(
        input.is_string(),
        "Final tool input should be a string, got: {:?}",
        input
    );
    assert_eq!(
        input,
        r#"Added the value "Add" to cell A1 (row 0, col 0) in the first sheet using the appropriate setValue BlinkOp."#
    );
}

#[test]
fn test_xml_final_with_empty_valid_tools() {
    // Test final tool with empty valid_tool_names list (real-world scenario)
    let parser = XmlParser::new(vec![]); // Empty valid tools list
    let xml_content = r#"<final>Added the value "Add" to cell A1 (row 0, col 0) in the first sheet using the appropriate setValue BlinkOp.</final>"#;

    let result = parser.parse(xml_content);

    assert!(
        result.is_ok(),
        "Final tool XML should parse successfully with empty valid tools"
    );

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1, "Should parse 1 tool call");
    assert_eq!(tool_calls[0].tool_name, "final");

    let input = &tool_calls[0].input;

    // This should NOT be an empty object
    if input.is_object() && input.as_object().unwrap().is_empty() {
        panic!(
            "Final tool with empty valid tools received empty object {{}} instead of string content!"
        );
    }

    // Should be a string
    assert!(
        input.is_string(),
        "Final tool input should be a string, got: {:?}",
        input
    );
    assert_eq!(
        input,
        r#"Added the value "Add" to cell A1 (row 0, col 0) in the first sheet using the appropriate setValue BlinkOp."#
    );
}

#[test]
fn test_xml_call_search_agent_structured_parameter() {
    // Test the expected structured parameter format for call_search_agent
    let parser = XmlParser::new(vec!["call_search_agent".to_string()]);
    let xml_content = r#"<call_search_agent>
<task>Find the top 10 children's books with their titles and authors as a list of [Title, Author].</task>
</call_search_agent>"#;

    let result = parser.parse(xml_content);
    assert!(
        result.is_ok(),
        "Structured parameter XML should parse successfully"
    );

    let tool_calls = result.unwrap();
    assert_eq!(tool_calls.len(), 1, "Should parse 1 tool call");
    assert_eq!(tool_calls[0].tool_name, "call_search_agent");

    let input = &tool_calls[0].input;

    // Should be an object with a 'task' parameter
    assert!(input.is_object(), "Input should be an object");
    let obj = input.as_object().unwrap();
    assert!(obj.contains_key("task"), "Should have 'task' parameter");
    assert_eq!(
        obj["task"],
        "Find the top 10 children's books with their titles and authors as a list of [Title, Author]."
    );
}

#[test]
fn test_xml_format_name() {
    let parser = XmlParser::new(TestData::get_builtin_tool_names());
    assert_eq!(parser.format_name(), "XML");
}

#[test]
fn test_xml_example_usage() {
    let parser = XmlParser::new(TestData::get_builtin_tool_names());
    let example = parser.example_usage();
    assert!(example.contains("<search>"));
    assert!(example.contains("<search>"));
    assert!(example.contains("<query>"));
}
