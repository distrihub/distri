//! Comprehensive tests for all tool call parser formats

use serde_json::json;

/// Test data for all parsers
pub struct TestData;

impl TestData {
    /// Get test data for XML format
    pub fn xml_valid() -> &'static str {
        r#"<search>
<query>test query</query>
<limit>5</limit>
</search>

<final>
<message>All done!</message>
</final>"#
    }

    /// Get complex XML test data
    pub fn xml_complex() -> &'static str {
        r#"<create_file>
<path>/tmp/test.txt</path>
<content>Hello world!</content>
<permissions>644</permissions>
</create_file>

<database_query>
<table>users</table>
<filters>{"active": true, "role": "admin"}</filters>
<limit>10</limit>
</database_query>"#
    }

    /// Get invalid XML (malformed)
    pub fn xml_invalid() -> &'static str {
        r#"<search>
<query>test query</query>
<limit>5
</search>"#
    }

    /// Get test data for JSONL format
    pub fn jsonl_valid() -> &'static str {
        r#"```tool_calls
{"name":"search","arguments":{"query":"test query","limit":5}}
{"name":"final","arguments":{"message":"All done!"}}
```"#
    }

    /// Get complex JSONL test data
    pub fn jsonl_complex() -> &'static str {
        r#"```tool_calls
{"name":"create_file","arguments":{"path":"/tmp/test.txt","content":"Hello world!","permissions":644}}
{"name":"database_query","arguments":{"table":"users","filters":{"active":true,"role":"admin"},"limit":10}}
```"#
    }

    /// Get invalid JSONL (malformed)
    pub fn jsonl_invalid() -> &'static str {
        r#"```tool_calls
{"name":"search","arguments":{"query":"test query","limit":5}}
{"name":"final","arguments":{"message":"unclosed string}
```"#
    }

    /// Expected tool calls for valid test cases
    pub fn expected_tool_calls() -> Vec<(&'static str, serde_json::Value)> {
        vec![
            ("search", json!({"query": "test query", "limit": 5})),
            ("final", json!({"message": "All done!"})),
        ]
    }

    /// Expected complex tool calls
    pub fn expected_complex_tool_calls() -> Vec<(&'static str, serde_json::Value)> {
        vec![
            (
                "create_file",
                json!({"path": "/tmp/test.txt", "content": "Hello world!", "permissions": 644}),
            ),
            (
                "database_query",
                json!({"table": "users", "filters": {"active": true, "role": "admin"}, "limit": 10}),
            ),
        ]
    }

    /// Get list of builtin tool names from distri
    pub fn get_builtin_tool_names() -> Vec<String> {
        vec![
            "final".to_string(),
            "search".to_string(),
            "transfer_to_agent".to_string(),
            "distri_execute_code".to_string(),
            "distri_crawl".to_string(),
            "distri_scrape".to_string(),
            "distri_browser".to_string(),
            "write_todos".to_string(),
            "create_file".to_string(),
            "database_query".to_string(),
        ]
    }
}

pub mod json_tests;
pub mod xml_tests;
