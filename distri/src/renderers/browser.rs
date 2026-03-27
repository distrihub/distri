use crate::printer::{COLOR_CYAN, COLOR_GRAY, COLOR_RESET};
use crate::renderers::RESULT_PREFIX;
use distri_types::{Part, ToolResponse};

/// Render browsr_scrape / browsr_crawl results.
pub fn render_scrape(result: &ToolResponse) {
    for part in &result.parts {
        match part {
            Part::Data(value) => {
                if let Some(obj) = value.as_object() {
                    if let Some(url) = obj.get("url").and_then(|u| u.as_str()) {
                        println!("{}{}scraped: {}{}", COLOR_CYAN, RESULT_PREFIX, url, COLOR_RESET);
                    }
                    if let Some(markdown) = obj.get("markdown").and_then(|m| m.as_str()) {
                        let total = markdown.lines().filter(|l| !l.trim().is_empty()).count();
                        println!(
                            "{}{}({} lines){}",
                            COLOR_GRAY, RESULT_PREFIX, total, COLOR_RESET
                        );
                    } else if let Some(title) = obj.get("title").and_then(|t| t.as_str()) {
                        println!("{}{}{}{}", COLOR_GRAY, RESULT_PREFIX, title, COLOR_RESET);
                    }
                } else {
                    crate::renderers::data::render_data_compact(value);
                }
            }
            Part::Text(text) => {
                let first = text.lines().next().unwrap_or("");
                println!("{}{}{}{}", COLOR_GRAY, RESULT_PREFIX, first, COLOR_RESET);
            }
            _ => {}
        }
    }
}

/// Render browsr_browser / browser_step results.
pub fn render_browser_step(result: &ToolResponse) {
    for part in &result.parts {
        match part {
            Part::Text(text) => {
                let first = text.lines().next().unwrap_or("");
                println!("{}{}{}{}", COLOR_GRAY, RESULT_PREFIX, first, COLOR_RESET);
            }
            Part::Data(value) => {
                if let Some(obj) = value.as_object() {
                    if let Some(url) = obj.get("url").and_then(|u| u.as_str()) {
                        println!("{}{}url: {}{}", COLOR_GRAY, RESULT_PREFIX, url, COLOR_RESET);
                    }
                    if let Some(status) = obj.get("status").and_then(|s| s.as_str()) {
                        println!(
                            "{}{}{}{}",
                            COLOR_GRAY, RESULT_PREFIX, status, COLOR_RESET
                        );
                    }
                }
            }
            Part::Image(file) => {
                let label = match file {
                    distri_types::FileType::Bytes { name, .. } => {
                        name.as_deref().unwrap_or("screenshot").to_string()
                    }
                    distri_types::FileType::Url { url, .. } => url.clone(),
                };
                println!(
                    "{}{}screenshot: {}{}",
                    COLOR_GRAY, RESULT_PREFIX, label, COLOR_RESET
                );
            }
            _ => {}
        }
    }
}
