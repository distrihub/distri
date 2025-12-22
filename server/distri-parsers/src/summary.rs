// Tool Summary Generation - responsible for generating tool summaries and descriptions

use distri_types::ToolCallFormat;
use distri_types::ToolDefinition;
use serde_json::Value;

/// Get format-specific instructions for the LLM, including available tools
pub fn get_available_tools(tool_defs: &[ToolDefinition]) -> String {
    let mut instructions = String::new();
    // Add detailed tool documentation
    instructions.push_str("\nAVAILABLE TOOLS:\n\n");

    for tool_def in tool_defs {
        instructions.push_str(&format!("## {}\n", tool_def.name));
        instructions.push_str(&format!("**Description:** {}\n\n", tool_def.description));

        // Parameters with schema info
        if let Some(properties) = tool_def
            .parameters
            .get("properties")
            .and_then(|p| p.as_object())
        {
            // Handle object parameters (with properties)
            instructions.push_str("**Parameters:**\n");

            let required = tool_def
                .parameters
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();

            for (param_name, schema) in properties {
                let is_required = required.contains(&param_name.as_str());
                let param_type = get_type_info(schema);
                let description = schema
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("No description provided");

                instructions.push_str(&format!(
                    "- **{}** ({}): `{}`\n",
                    param_name,
                    if is_required { "required" } else { "optional" },
                    param_type
                ));
                instructions.push_str(&format!("  {}\n", description));

                instructions.push_str("\n");
            }
        } else if tool_def.parameters.get("type").and_then(|t| t.as_str()) == Some("string") {
            // Handle direct string parameters (like final tool)
            let description = tool_def
                .parameters
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("No description provided");

            instructions.push_str("**Parameters:**\n");
            instructions.push_str(&format!("- Direct string input: `{}`\n\n", description));
        } else {
            // Handle tools with no parameters
            instructions.push_str("**Parameters:** None\n\n");
        }

        // Add examples if available
        if let Some(examples) = &tool_def.examples {
            instructions.push_str("**Examples:**\n");
            instructions.push_str(examples);
            instructions.push_str("\n");
        }
        instructions.push_str("---");
    }

    instructions
}

/// Get enhanced type information including nested structures
fn get_type_info(schema: &Value) -> String {
    // Handle Option<T> types (anyOf with null)
    if let Some(any_of) = schema.get("anyOf").and_then(|a| a.as_array()) {
        // Look for the pattern [{"type": "null"}, {"type": "actual_type"}] or similar
        if any_of.len() == 2 {
            let non_null_schema = any_of
                .iter()
                .find(|s| s.get("type").and_then(|t| t.as_str()) != Some("null"));

            if let Some(inner_schema) = non_null_schema {
                return get_type_info(inner_schema);
            }
        }
    }

    // Handle complex schemas with oneOf/allOf - return generic type for LLM simplicity
    if schema.get("oneOf").is_some() || schema.get("allOf").is_some() {
        return "object (complex schema - see examples)".to_string();
    }

    match schema.get("type").and_then(|t| t.as_str()) {
        Some("object") => {
            if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
                let prop_types: Vec<String> = properties
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, get_type_info(v)))
                    .collect();
                format!("object {{ {} }}", prop_types.join(", "))
            } else {
                "object".to_string()
            }
        }
        Some("array") => {
            if let Some(items) = schema.get("items") {
                format!("array<{}>", get_type_info(items))
            } else {
                "array".to_string()
            }
        }
        Some("string") => {
            if let Some(enum_vals) = schema.get("enum").and_then(|e| e.as_array()) {
                let variants: Vec<String> = enum_vals
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| format!("\"{}\"", s))
                    .collect();
                format!("enum({})", variants.join(" | "))
            } else {
                "string".to_string()
            }
        }
        Some(t) => t.to_string(),
        None => "any".to_string(),
    }
}

/// Generate XML tags recursively for nested objects
fn generate_xml_for_value(key: &str, value: &Value) -> String {
    match value {
        Value::Object(obj) => {
            let mut xml = format!("<{}>\n", key);
            for (nested_key, nested_value) in obj {
                xml.push_str(&generate_xml_for_value(nested_key, nested_value));
            }
            xml.push_str(&format!("</{}>\n", key));
            xml
        }
        Value::Array(arr) => {
            let mut xml = String::new();
            for item in arr.iter() {
                xml.push_str(&generate_xml_for_value(key, item));
            }
            xml
        }
        _ => {
            let value_str = match value {
                Value::String(s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                Value::Null => "null".to_string(),
                _ => value.to_string(),
            };
            format!("<{}>{}</{}>\n", key, value_str, key)
        }
    }
}

/// Generate usage examples from a list of example Values based on format
pub fn generate_usage_examples_from_values(
    tool_name: &str,
    examples: &[(String, Value)],
    format: &ToolCallFormat,
) -> String {
    let mut output = String::new();

    for (i, (header, example)) in examples.iter().enumerate() {
        if i > 0 {
            output.push_str("\n");
        }
        output.push_str(&format!("### {}\n", header));

        match format {
            ToolCallFormat::Xml => {
                // New streaming XML format: <tool_name><param>value</param></tool_name>
                output.push_str(&format!("<{}>", tool_name));

                if let Value::Object(obj) = example {
                    for (key, value) in obj {
                        output.push_str(&generate_xml_for_value(key, value));
                    }
                } else {
                    // Handle non-object examples (string parameters) directly
                    let value_str = match example {
                        Value::String(s) => s.clone(),
                        Value::Bool(b) => b.to_string(),
                        Value::Number(n) => n.to_string(),
                        Value::Null => "null".to_string(),
                        _ => example.to_string(),
                    };
                    output.push_str(&value_str);
                }

                output.push_str(&format!("</{}>\n", tool_name));
            }
            ToolCallFormat::JsonL => {
                // JSONL format: ```tool_calls\n{"name":"tool","arguments":{"param":"value"}}```
                output.push_str("```tool_calls\n");
                let tool_call = serde_json::json!({
                    "name": tool_name,
                    "arguments": example
                });
                output.push_str(
                    &serde_json::to_string(&tool_call).unwrap_or_else(|_| "{}".to_string()),
                );
                output.push_str("\n```\n");
            }
            ToolCallFormat::Code => {
                output.push_str("```js\n");
                output.push_str(&format!("await {}(", tool_name));
                let args_json =
                    serde_json::to_string_pretty(example).unwrap_or_else(|_| "{}".to_string());
                output.push_str(&args_json);
                output.push_str(");\n");
                output.push_str("```\n");
            }
            _ => {}
        }
    }

    output
}
