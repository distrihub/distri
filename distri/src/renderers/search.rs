use crate::printer::{COLOR_CYAN, COLOR_GRAY, COLOR_RESET};
use distri_types::{Part, ToolResponse};

/// Render search tool results — shows titles and URLs from the data array.
pub fn render_search(result: &ToolResponse) {
    for part in &result.parts {
        match part {
            Part::Data(value) => {
                if let Some(obj) = value.as_object() {
                    if let Some(serde_json::Value::Array(items)) = obj.get("data") {
                        let count = items.len();
                        println!(
                            "{}  {} result{}{}",
                            COLOR_CYAN,
                            count,
                            if count == 1 { "" } else { "s" },
                            COLOR_RESET
                        );
                        for (i, item) in items.iter().take(5).enumerate() {
                            let title = item
                                .get("title")
                                .and_then(|t| t.as_str())
                                .unwrap_or("(untitled)");
                            let url = item.get("url").and_then(|u| u.as_str()).unwrap_or("");
                            println!("{}  {}. {}{}", COLOR_GRAY, i + 1, title, COLOR_RESET);
                            if !url.is_empty() {
                                println!("{}     {}{}", COLOR_GRAY, url, COLOR_RESET);
                            }
                        }
                        if count > 5 {
                            println!(
                                "{}  … and {} more{}",
                                COLOR_GRAY,
                                count - 5,
                                COLOR_RESET
                            );
                        }
                        continue;
                    }
                }
                // Fallback for non-standard data
                crate::renderers::data::render_data_compact(value);
            }
            Part::Text(text) => {
                let preview: Vec<&str> = text.lines().take(2).collect();
                println!("{}  {}{}", COLOR_GRAY, preview.join("\n  "), COLOR_RESET);
            }
            _ => {}
        }
    }
}
