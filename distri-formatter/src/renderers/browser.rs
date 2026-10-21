use super::RESULT_PREFIX;
use crate::colors::{COLOR_CYAN, COLOR_GRAY, COLOR_RESET};
use distri_types::{Part, ToolResponse};
use std::io::Write;

/// Render browsr_scrape / browsr_crawl results.
pub fn render_scrape(out: &mut dyn Write, result: &ToolResponse) {
    for part in &result.parts {
        match part {
            Part::Data(value) => {
                if let Some(obj) = value.as_object() {
                    if let Some(url) = obj.get("url").and_then(|u| u.as_str()) {
                        outln!(
                            out,
                            "{}{}scraped: {}{}",
                            COLOR_CYAN,
                            RESULT_PREFIX,
                            url,
                            COLOR_RESET
                        );
                    }
                    if let Some(markdown) = obj.get("markdown").and_then(|m| m.as_str()) {
                        let total = markdown.lines().filter(|l| !l.trim().is_empty()).count();
                        outln!(
                            out,
                            "{}{}({} lines){}",
                            COLOR_GRAY,
                            RESULT_PREFIX,
                            total,
                            COLOR_RESET
                        );
                    } else if let Some(title) = obj.get("title").and_then(|t| t.as_str()) {
                        outln!(
                            out,
                            "{}{}{}{}",
                            COLOR_GRAY,
                            RESULT_PREFIX,
                            title,
                            COLOR_RESET
                        );
                    }
                } else {
                    crate::renderers::data::render_data_compact(out, value);
                }
            }
            Part::Text(text) => {
                let first = text.lines().next().unwrap_or("");
                outln!(
                    out,
                    "{}{}{}{}",
                    COLOR_GRAY,
                    RESULT_PREFIX,
                    first,
                    COLOR_RESET
                );
            }
            _ => {}
        }
    }
}

/// Render browsr_browser / browser_step results.
pub fn render_browser_step(out: &mut dyn Write, result: &ToolResponse) {
    for part in &result.parts {
        match part {
            Part::Text(text) => {
                let first = text.lines().next().unwrap_or("");
                outln!(
                    out,
                    "{}{}{}{}",
                    COLOR_GRAY,
                    RESULT_PREFIX,
                    first,
                    COLOR_RESET
                );
            }
            Part::Data(value) => {
                if let Some(obj) = value.as_object() {
                    if let Some(url) = obj.get("url").and_then(|u| u.as_str()) {
                        outln!(
                            out,
                            "{}{}url: {}{}",
                            COLOR_GRAY,
                            RESULT_PREFIX,
                            url,
                            COLOR_RESET
                        );
                    }
                    if let Some(status) = obj.get("status").and_then(|s| s.as_str()) {
                        outln!(
                            out,
                            "{}{}{}{}",
                            COLOR_GRAY,
                            RESULT_PREFIX,
                            status,
                            COLOR_RESET
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
                outln!(
                    out,
                    "{}{}screenshot: {}{}",
                    COLOR_GRAY,
                    RESULT_PREFIX,
                    label,
                    COLOR_RESET
                );
            }
            _ => {}
        }
    }
}
