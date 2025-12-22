use distri_types::ToolDefinition;

pub fn get_tool_descriptions(tool_defs: &[ToolDefinition]) -> String {
    let mut out = String::new();
    for tool_def in tool_defs.iter() {
        let name = tool_def.name.clone();
        let description = tool_def.description.clone();
        let params = tool_def.parameters.clone();
        // Print tool name and description
        out.push_str(&format!("Tool: {}\nDescription: {}\n", name, description));
        // Print parameters
        out.push_str("Parameters:\n");

        if let Some(props) = params.get("properties").and_then(|p| p.as_object()) {
            let required = params
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();
            for (param, schema) in props.iter() {
                let is_required = required.contains(&param.as_str());
                let typ = schema.get("type").and_then(|t| t.as_str()).unwrap_or("any");
                let desc = schema
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");
                out.push_str(&format!(
                    "  • {} ({}): {}\n    {}\n",
                    param,
                    if is_required { "required" } else { "optional" },
                    typ,
                    desc
                ));
            }
        }

        out.push_str("\n");
    }

    out
}
