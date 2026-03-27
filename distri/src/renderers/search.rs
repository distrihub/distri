use crate::printer::{COLOR_CYAN, COLOR_GRAY, COLOR_RESET};
use crate::renderers::RESULT_PREFIX;
use distri_types::{Part, ToolResponse};

/// Render search tool results.
pub fn render_search(result: &ToolResponse) {
    for part in &result.parts {
        match part {
            Part::Data(value) => {
                if let Some(obj) = value.as_object() {
                    if let Some(serde_json::Value::Array(items)) = obj.get("data") {
                        let count = items.len();
                        println!(
                            "{}{}{} result{}{}",
                            COLOR_CYAN,
                            RESULT_PREFIX,
                            count,
                            if count == 1 { "" } else { "s" },
                            COLOR_RESET
                        );
                        for (i, item) in items.iter().take(3).enumerate() {
                            let title = item
                                .get("title")
                                .and_then(|t| t.as_str())
                                .unwrap_or("(untitled)");
                            println!("{}  {}. {}{}", COLOR_GRAY, i + 1, title, COLOR_RESET);
                        }
                        if count > 3 {
                            println!(
                                "{}  … and {} more{}",
                                COLOR_GRAY,
                                count - 3,
                                COLOR_RESET
                            );
                        }
                        continue;
                    }
                }
                crate::renderers::data::render_data_compact(value);
            }
            Part::Text(text) => {
                let first = text.lines().next().unwrap_or("");
                println!("{}{}{}{}", COLOR_GRAY, RESULT_PREFIX, first, COLOR_RESET);
            }
            _ => {}
        }
    }
}
