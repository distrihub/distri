use crate::printer::{COLOR_GRAY, COLOR_RESET};
use crate::renderers::data::render_data_compact;
use distri_types::{Part, ToolResponse};

/// Generic part-by-part tool result rendering (fallback for unrecognized tools).
pub fn render_tool_result(result: &ToolResponse) {
    for part in &result.parts {
        match part {
            Part::Text(text) => {
                let lines: Vec<&str> = text.lines().take(2).collect();
                let preview = lines.join("\n");
                if text.lines().count() > 2 {
                    println!("{}  {}\n  …{}", COLOR_GRAY, preview, COLOR_RESET);
                } else {
                    println!("{}  {}{}", COLOR_GRAY, preview, COLOR_RESET);
                }
            }
            Part::Data(value) => {
                render_data_compact(value);
            }
            Part::Artifact(meta) => {
                println!(
                    "{}  artifact: {} ({}){}",
                    COLOR_GRAY,
                    meta.original_filename
                        .as_deref()
                        .unwrap_or(&meta.relative_path),
                    meta.content_type.as_deref().unwrap_or("unknown"),
                    COLOR_RESET,
                );
            }
            Part::Image(file) => {
                let label = match file {
                    distri_types::FileType::Bytes {
                        name, mime_type, ..
                    } => {
                        format!("{} ({})", name.as_deref().unwrap_or("image"), mime_type)
                    }
                    distri_types::FileType::Url { url, .. } => url.clone(),
                };
                println!("{}  image: {}{}", COLOR_GRAY, label, COLOR_RESET);
            }
            _ => {
                println!("{}  [{}]{}", COLOR_GRAY, part.type_name(), COLOR_RESET);
            }
        }
    }
}
