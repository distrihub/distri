use crate::printer::{COLOR_GRAY, COLOR_RESET};

/// Render a Data part compactly — handles search results, markdown, generic JSON.
pub fn render_data_compact(value: &serde_json::Value) {
    if let Some(obj) = value.as_object() {
        // Object with "data" array (search results pattern)
        if let Some(serde_json::Value::Array(items)) = obj.get("data") {
            let count = items.len();
            for (i, item) in items.iter().take(3).enumerate() {
                if let Some(title) = item.get("title").and_then(|t| t.as_str()) {
                    let url = item.get("url").and_then(|u| u.as_str()).unwrap_or("");
                    println!("{}  {}. {}{}", COLOR_GRAY, i + 1, title, COLOR_RESET);
                    if !url.is_empty() {
                        println!("{}     {}{}", COLOR_GRAY, url, COLOR_RESET);
                    }
                } else {
                    let s = serde_json::to_string(item).unwrap_or_default();
                    let preview = if s.len() > 120 {
                        format!("{}…", &s[..120])
                    } else {
                        s
                    };
                    println!("{}  {}. {}{}", COLOR_GRAY, i + 1, preview, COLOR_RESET);
                }
            }
            if count > 3 {
                println!("{}  … and {} more{}", COLOR_GRAY, count - 3, COLOR_RESET);
            }
            return;
        }

        // Object with markdown field
        if let Some(markdown) = obj.get("markdown").and_then(|m| m.as_str()) {
            let lines: Vec<&str> = markdown
                .lines()
                .filter(|l| !l.trim().is_empty())
                .take(3)
                .collect();
            for line in &lines {
                println!("{}  {}{}", COLOR_GRAY, line, COLOR_RESET);
            }
            let total = markdown.lines().filter(|l| !l.trim().is_empty()).count();
            if total > 3 {
                println!("{}  …{}", COLOR_GRAY, COLOR_RESET);
            }
            return;
        }
    }

    // Fallback: compact single-line JSON
    let s = serde_json::to_string(value).unwrap_or_default();
    let preview = if s.len() > 150 {
        format!("{}…", &s[..150])
    } else {
        s
    };
    println!("{}  {}{}", COLOR_GRAY, preview, COLOR_RESET);
}
