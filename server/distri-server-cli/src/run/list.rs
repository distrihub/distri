use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, ContentArrangement, Table};
use distri_core::agent::AgentOrchestrator;
use distri_types::auth::AuthMetadata;
use std::sync::Arc;

pub async fn list(executor: Arc<AgentOrchestrator>) -> anyhow::Result<()> {
    let (agents, _) = executor.list_agents(None, None).await;
    let mut table = Table::new()
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .to_owned();

    table.add_row(vec!["Agent", "Description", "Tools"]);
    for agent_config in agents.iter() {
        // Only get tools for StandardAgent (others have built-in tools)
        let tools = match agent_config {
            distri_types::configuration::AgentConfig::StandardAgent(def) => {
                executor.get_agent_tools(def, &Arc::default()).await?
            }
            _ => vec![], // Workflow agents have built-in tools
        };
        let inner = tools_table(&tools);

        table.add_row(vec![
            agent_config.get_name().to_string(),
            agent_config.get_description().to_string(),
            inner,
        ]);
    }
    println!("{table}");
    Ok(())
}

pub async fn list_tools(
    executor: Arc<AgentOrchestrator>,
    filter: Option<String>,
) -> anyhow::Result<()> {
    let mut tools = executor.get_all_available_tools().await?;

    if let Some(plugin_filter) = filter {
        let normalize = |value: &str| value.replace('-', "_").to_lowercase();
        let normalized_filter = normalize(&plugin_filter);
        tools.retain(|tool| {
            tool.get_plugin_name()
                .map(|name| normalize(&name) == normalized_filter)
                .unwrap_or(false)
        });
    }

    tools.sort_by_key(|tool| {
        let plugin = tool.get_plugin_name().unwrap_or_default();
        (plugin, tool.get_name())
    });

    tools.dedup_by(|a, b| {
        let a_key = (a.get_plugin_name().unwrap_or_default(), a.get_name());
        let b_key = (b.get_plugin_name().unwrap_or_default(), b.get_name());
        a_key == b_key
    });

    // Print detailed tools information
    for (i, tool) in tools.iter().enumerate() {
        let is_last = i == tools.len() - 1;
        let connector = if is_last { "└─" } else { "├─" };

        // Tool name and description
        let display_name = if let Some(plugin_name) = tool.get_plugin_name() {
            format!("{}/{}", plugin_name, tool.get_name())
        } else {
            tool.get_name()
        };
        println!("│ {} 🔧 {}", connector, display_name);

        let prefix = "";

        // Description
        let description = tool.get_description();
        if !description.is_empty() {
            println!("│ {}    💬 {}", prefix, description);
        }

        // Parameters
        let parameters = tool.get_parameters();
        let param_info = format_parameters_clean(&parameters);
        if !param_info.is_empty() {
            println!("│ {}    📝 {}", prefix, param_info);
        }

        // Auth requirements
        if let Some(auth_metadata) = tool.get_auth_metadata() {
            let auth_info = format_auth_clean(auth_metadata.as_ref());
            println!("│ {}    🔐 {}", prefix, auth_info);
        }

        if !is_last {
            println!("│ {}", prefix);
        }
    }

    println!();
    Ok(())
}

fn tools_table(tools: &Vec<Arc<dyn distri_core::tools::Tool>>) -> String {
    let mut inner_table = Table::new()
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .to_owned();
    inner_table.add_row(vec!["Tool", "Description", "Parameters", "Auth"]);

    tools.iter().for_each(|t| {
        let description = t.get_description().clone();

        // Get parameter schema
        let parameters = t.get_parameters();
        let param_info = format_parameter_schema(&parameters);

        // Get auth requirements
        let auth_info = if let Some(auth_metadata) = t.get_auth_metadata() {
            let auth_type = auth_metadata.get_auth_type();
            match auth_type {
                distri_types::auth::AuthType::OAuth2 { scopes, .. } => {
                    let entity = auth_metadata.get_auth_entity();
                    if scopes.is_empty() {
                        format!("🔐 {}", entity)
                    } else {
                        format!("🔐 {} ({})", entity, scopes.join(", "))
                    }
                }
                distri_types::auth::AuthType::Secret { provider, fields } => {
                    let field_list = if fields.is_empty() {
                        "secret".to_string()
                    } else {
                        fields
                            .iter()
                            .map(|f| f.key.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    format!("🔑 {} ({})", provider, field_list)
                }
                distri_types::auth::AuthType::None => "None".to_string(),
            }
        } else {
            "None".to_string()
        };
        let truncated_desc = if description.len() > 60 {
            format!("{}...", &description[..57])
        } else {
            description
        };

        inner_table.add_row(vec![t.get_name(), truncated_desc, param_info, auth_info]);
    });
    inner_table.to_string()
}

/// Format parameter schema into a readable string
fn format_parameter_schema(parameters: &serde_json::Value) -> String {
    if let Some(obj) = parameters.as_object() {
        if let Some(properties) = obj.get("properties") {
            if let Some(props_obj) = properties.as_object() {
                let mut required_params = Vec::new();
                let mut optional_params = Vec::new();

                // Get required fields
                let required_fields: Vec<String> = obj
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();

                for (param_name, param_schema) in props_obj {
                    let param_type = param_schema
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("any");

                    let param_info = if param_type == "object" {
                        format!("{}: {{}}", param_name)
                    } else if param_type == "array" {
                        format!("{}: []", param_name)
                    } else {
                        format!("{}: {}", param_name, param_type)
                    };

                    if required_fields.contains(param_name) {
                        required_params.push(param_info);
                    } else {
                        optional_params.push(format!("({})", param_info));
                    }
                }

                let mut result = Vec::new();
                if !required_params.is_empty() {
                    result.extend(required_params);
                }
                if !optional_params.is_empty() {
                    result.extend(optional_params);
                }

                if result.is_empty() {
                    "No parameters".to_string()
                } else {
                    result.join(", ")
                }
            } else {
                "No parameters".to_string()
            }
        } else {
            "No parameters".to_string()
        }
    } else {
        "No parameters".to_string()
    }
}

/// Format parameters in a clean, readable way for the new list format
fn format_parameters_clean(parameters: &serde_json::Value) -> String {
    if let Some(obj) = parameters.as_object() {
        if let Some(properties) = obj.get("properties") {
            if let Some(props_obj) = properties.as_object() {
                let required_fields: Vec<String> = obj
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();

                let mut param_strings = Vec::new();
                for (param_name, param_schema) in props_obj {
                    let param_type = param_schema
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("any");

                    let description = param_schema
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("");

                    let required_marker = if required_fields.contains(param_name) {
                        "●"
                    } else {
                        "○"
                    };

                    if description.is_empty() {
                        param_strings.push(format!(
                            "{} {}: {}",
                            required_marker, param_name, param_type
                        ));
                    } else {
                        param_strings.push(format!(
                            "{} {}: {} ({})",
                            required_marker, param_name, param_type, description
                        ));
                    }
                }

                if param_strings.is_empty() {
                    "".to_string()
                } else {
                    format!("Parameters: {}", param_strings.join(", "))
                }
            } else {
                "".to_string()
            }
        } else {
            "".to_string()
        }
    } else {
        "".to_string()
    }
}

/// Format auth information cleanly
fn format_auth_clean(auth_metadata: &dyn AuthMetadata) -> String {
    let auth_type = auth_metadata.get_auth_type();
    match auth_type {
        distri_types::auth::AuthType::OAuth2 { scopes, .. } => {
            let entity = auth_metadata.get_auth_entity();
            if scopes.is_empty() {
                format!("OAuth2 with {}", entity)
            } else {
                format!("OAuth2 with {} (scopes: {})", entity, scopes.join(", "))
            }
        }
        distri_types::auth::AuthType::Secret { provider, fields } => {
            if fields.is_empty() {
                format!("Secret for {}", provider)
            } else {
                let field_list = fields
                    .iter()
                    .map(|f| f.key.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Secret for {} (keys: {})", provider, field_list)
            }
        }
        distri_types::auth::AuthType::None => "Uses environment variables/secrets".to_string(),
    }
}
