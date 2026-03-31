//! Human-readable status text generation from tool calls.
//!
//! Shared across all surfaces — produces strings like
//! "Checking calendar via Google Calendar..." from tool name + input.

use serde_json::Value;

/// Produce a human-readable status description for a tool call.
///
/// Used by all surfaces to show what the agent is currently doing.
pub fn format_status_text(name: &str, input: &Value) -> String {
    let str_field = |key: &str| -> Option<String> {
        input.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
    };

    let truncate = |s: &str, max: usize| -> String {
        if s.len() > max {
            format!("{}...", &s[..max])
        } else {
            s.to_string()
        }
    };

    match name {
        "distri_request" => {
            let method = str_field("method").unwrap_or_default();
            let path = str_field("path").unwrap_or_default();

            // Try to resolve a friendly name from x-connection-id header
            let connection_name = input
                .get("headers")
                .and_then(|h| h.get("x-connection-id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            if method == "GET" && path.contains("/connections") {
                "Checking available connections...".to_string()
            } else if let Some(conn) = connection_name {
                let service = friendly_service_name(&path);
                format!("{} via {}", service, conn)
            } else if method == "GET" && path.contains("/calendar") {
                "Checking calendar...".to_string()
            } else if method == "GET" && path.contains("/email") {
                "Checking email...".to_string()
            } else if method == "GET" {
                format!("Fetching {}...", truncate(&path, 40))
            } else if method == "POST" {
                format!("Sending request to {}...", truncate(&path, 40))
            } else {
                format!("{} {}...", method, truncate(&path, 40))
            }
        }
        "execute_shell" => {
            let cmd = str_field("command").unwrap_or_else(|| "...".to_string());
            format!("Running command: {}", truncate(&cmd, 50))
        }
        "search" => {
            let query = str_field("query").unwrap_or_else(|| "...".to_string());
            format!("Searching: {}", truncate(&query, 50))
        }
        "browsr_scrape" => {
            let url = str_field("url").unwrap_or_else(|| "...".to_string());
            let host = extract_host(&url);
            format!("Browsing {}", host)
        }
        "browsr_crawl" => {
            let url = str_field("url").unwrap_or_else(|| "...".to_string());
            let host = extract_host(&url);
            format!("Crawling {}", host)
        }
        "browsr_browser" | "browser_step" => {
            let action = str_field("action").unwrap_or_else(|| "navigating".to_string());
            format!("Browser: {}", action)
        }
        "load_skill" => {
            let skill = str_field("skill_name").unwrap_or_else(|| "...".to_string());
            format!("Loading skill: {}", skill)
        }
        "run_skill_script" => {
            let skill = str_field("skill_name").unwrap_or_else(|| "...".to_string());
            format!("Running skill: {}", skill)
        }
        "read_file" => {
            let path = str_field("path")
                .or_else(|| str_field("file_path"))
                .unwrap_or_else(|| "...".to_string());
            format!("Reading {}", truncate(&path, 50))
        }
        "write_file" => {
            let path = str_field("path")
                .or_else(|| str_field("file_path"))
                .unwrap_or_else(|| "...".to_string());
            format!("Writing {}", truncate(&path, 50))
        }
        "transfer_to_agent" => {
            let agent = str_field("agent_name").unwrap_or_else(|| "...".to_string());
            format!("Handing off to {}", agent)
        }
        "tool_search" => "Searching tools...".to_string(),
        "inject_connection_env" => {
            let provider = str_field("provider_name").unwrap_or_else(|| "service".to_string());
            format!("Connecting to {}...", provider)
        }
        "create_skill" => {
            let skill = str_field("name")
                .or_else(|| str_field("skill_name"))
                .unwrap_or_else(|| "...".to_string());
            format!("Creating skill: {}", skill)
        }
        "start_shell" => "Starting shell session...".to_string(),
        "stop_shell" => "Stopping shell session...".to_string(),
        _ => {
            // Fallback: humanize the tool name
            let readable = name.replace('_', " ");
            format!("{}...", capitalize_first(&readable))
        }
    }
}

/// Extract a friendly service name from an API path.
fn friendly_service_name(path: &str) -> String {
    if path.contains("/calendar") {
        "Checking calendar".to_string()
    } else if path.contains("/email") || path.contains("/mail") {
        "Checking email".to_string()
    } else if path.contains("/drive") || path.contains("/files") {
        "Checking files".to_string()
    } else if path.contains("/contacts") {
        "Checking contacts".to_string()
    } else if path.contains("/tasks") || path.contains("/todos") {
        "Checking tasks".to_string()
    } else {
        "Making request".to_string()
    }
}

/// Extract hostname from a URL, falling back to the URL itself.
fn extract_host(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|s| s.split('/').next())
        .unwrap_or(url)
        .to_string()
}

/// Capitalize the first letter of a string.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn distri_request_with_connection() {
        let input = json!({
            "method": "GET",
            "path": "/calendar/events",
            "headers": { "x-connection-id": "Google Calendar" }
        });
        let result = format_status_text("distri_request", &input);
        assert_eq!(result, "Checking calendar via Google Calendar");
    }

    #[test]
    fn execute_shell_status() {
        let input = json!({"command": "npm test"});
        let result = format_status_text("execute_shell", &input);
        assert_eq!(result, "Running command: npm test");
    }

    #[test]
    fn search_status() {
        let input = json!({"query": "rust async patterns"});
        let result = format_status_text("search", &input);
        assert_eq!(result, "Searching: rust async patterns");
    }

    #[test]
    fn browsr_scrape_status() {
        let input = json!({"url": "https://docs.rs/tokio/latest"});
        let result = format_status_text("browsr_scrape", &input);
        assert_eq!(result, "Browsing docs.rs");
    }

    #[test]
    fn transfer_to_agent_status() {
        let input = json!({"agent_name": "researcher"});
        let result = format_status_text("transfer_to_agent", &input);
        assert_eq!(result, "Handing off to researcher");
    }

    #[test]
    fn unknown_tool_fallback() {
        let input = json!({});
        let result = format_status_text("my_custom_tool", &input);
        assert_eq!(result, "My custom tool...");
    }
}
