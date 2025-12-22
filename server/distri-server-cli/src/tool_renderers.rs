use crate::shared_state::SharedState;
use crate::Cli;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::warn;

const COLOR_RESET: &str = "\x1b[0m";
const COLOR_DISTRI_HEADING: &str = "\x1b[38;2;0;124;145m";
const COLOR_GREEN: &str = "\x1b[32m";
const COLOR_RED: &str = "\x1b[31m";
const COLOR_GRAY: &str = "\x1b[90m";

pub type ToolRendererFn =
    Arc<dyn for<'a> Fn(ToolCallStartContext<'a>) -> Option<String> + Send + Sync>;
type ToolRendererHandler = for<'a> fn(ToolCallStartContext<'a>) -> Option<String>;

#[derive(Clone, Copy)]
pub struct ToolRendererContext<'a> {
    pub cli: &'a Cli,
    pub workspace_path: &'a Path,
    pub shared_state: Option<&'a SharedState>,
}

#[derive(Clone, Copy)]
pub struct ToolCallStartContext<'a> {
    pub tool_call_id: &'a str,
    pub tool_name: &'a str,
    pub input: &'a Value,
    pub workspace_path: &'a Path,
    pub shared_state: Option<&'a SharedState>,
}

#[derive(Clone, Copy)]
pub struct ToolCallFinishContext<'a> {
    pub tool_call_id: &'a str,
    pub tool_name: &'a str,
    pub input: &'a Value,
    pub result: Option<&'a Value>,
    pub workspace_path: &'a Path,
    pub shared_state: Option<&'a SharedState>,
    pub success: bool,
}

fn renderer_entry(name: &str, handler: ToolRendererHandler) -> (String, ToolRendererFn) {
    (
        name.to_string(),
        Arc::new(move |context: ToolCallStartContext<'_>| handler(context)),
    )
}

pub struct ToolRendererRegistry {
    renderers: HashMap<String, ToolRendererFn>,
    workspace_path: PathBuf,
    shared_state: Option<SharedState>,
}

impl ToolRendererRegistry {
    pub fn new(
        renderers: HashMap<String, ToolRendererFn>,
        workspace_path: PathBuf,
        shared_state: Option<SharedState>,
    ) -> Self {
        Self {
            renderers,
            workspace_path,
            shared_state,
        }
    }

    pub fn handle_tool_start(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        input: &Value,
    ) -> Vec<String> {
        let context = ToolCallStartContext {
            tool_call_id,
            tool_name,
            input,
            workspace_path: &self.workspace_path,
            shared_state: self.shared_state.as_ref(),
        };

        self.lookup_renderer(tool_name)
            .and_then(|renderer| renderer(context))
            .into_iter()
            .collect()
    }

    pub fn handle_tool_end(
        &self,
        _tool_call_id: &str,
        _tool_name: &str,
        _input: &Value,
        _result: Option<&Value>,
        _success: bool,
    ) -> Vec<String> {
        Vec::new()
    }

    fn lookup_renderer(&self, tool_name: &str) -> Option<&ToolRendererFn> {
        if let Some(renderer) = self.renderers.get(tool_name) {
            return Some(renderer);
        }

        if let Some((_, short)) = tool_name.rsplit_once('/') {
            if let Some(renderer) = self.renderers.get(short) {
                return Some(renderer);
            }
        }

        if let Some((_, short)) = tool_name.rsplit_once('.') {
            if let Some(renderer) = self.renderers.get(short) {
                return Some(renderer);
            }
        }

        None
    }
}

#[derive(Clone)]
struct DiffBlock {
    start_line: usize,
    search: Vec<String>,
    replace: Vec<String>,
}

pub fn default_tool_renderers() -> Vec<(String, ToolRendererFn)> {
    vec![
        renderer_entry("apply_diff", render_apply_diff),
        renderer_entry("fs_write_file", render_write_file),
        renderer_entry("write_to_file", render_write_file),
        renderer_entry("fs_read_file", render_read_file),
        renderer_entry("read_file", render_read_file),
        renderer_entry("fs_list_directory", render_list_directory),
        renderer_entry("fs_tree", render_tree),
        renderer_entry("fs_search_files", render_search_files),
        renderer_entry("fs_search_within_files", render_search_within_files),
        renderer_entry("fs_get_file_info", render_get_file_info),
        renderer_entry("fs_delete_file", render_delete_file),
        renderer_entry("fs_create_directory", render_create_directory),
        renderer_entry("fs_copy_file", render_copy_file),
        renderer_entry("fs_move_file", render_move_file),
    ]
}

fn render_apply_diff(context: ToolCallStartContext<'_>) -> Option<String> {
    let diff = context.input.get("diff")?.as_str()?.to_string();
    let path = context.input.get("path")?.as_str()?.to_string();

    if let Some(blocks) = parse_structured_apply_diff(&diff) {
        if blocks.is_empty() {
            return None;
        }
        let additions: usize = blocks.iter().map(|b| b.replace.len()).sum();
        let deletions: usize = blocks.iter().map(|b| b.search.len()).sum();
        let mut output = String::new();
        output.push_str(&format!(
            "{}Edited {} (+{} -{}){}\n",
            COLOR_DISTRI_HEADING, path, additions, deletions, COLOR_RESET
        ));
        output.push_str(&format!(
            "{}diff -- apply_diff {}{}\n",
            COLOR_GRAY, path, COLOR_RESET
        ));
        for block in &blocks {
            output.push_str(&format!(
                "{}@@ line {}@@{}\n",
                COLOR_GRAY, block.start_line, COLOR_RESET
            ));
            for line in &block.search {
                output.push_str(&format!("{}-{}{}\n", COLOR_RED, line, COLOR_RESET));
            }
            for line in &block.replace {
                output.push_str(&format!("{}+{}{}\n", COLOR_GREEN, line, COLOR_RESET));
            }
            output.push('\n');
        }
        Some(output)
    } else {
        render_unified_diff(&path, &diff)
    }
}

fn render_write_file(context: ToolCallStartContext<'_>) -> Option<String> {
    let path_value = context
        .input
        .get("path")
        .or_else(|| context.input.get("file_path"))?;
    let path = path_value.as_str()?.to_string();
    let content = context.input.get("content")?.as_str()?.to_string();

    let mut additions = 0usize;
    let mut output = String::new();
    for line in content.lines() {
        additions += 1;
        output.push_str(&format!("{}+{}{}\n", COLOR_GREEN, line, COLOR_RESET));
    }

    let header = format!(
        "{}Edited {} (+{} -0){}\n{}diff -- write {}{}\n",
        COLOR_DISTRI_HEADING, path, additions, COLOR_RESET, COLOR_GRAY, path, COLOR_RESET
    );

    Some(format!("{}{}", header, output))
}

fn render_unified_diff(path: &str, diff: &str) -> Option<String> {
    if diff.trim().is_empty() {
        return None;
    }

    let mut additions = 0usize;
    let mut deletions = 0usize;
    let mut rendered = String::new();

    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") {
            rendered.push_str(&format!("{}{}{}\n", COLOR_GRAY, line, COLOR_RESET));
            continue;
        }
        if line.starts_with("@@") {
            rendered.push_str(&format!("{}{}{}\n", COLOR_GRAY, line, COLOR_RESET));
            continue;
        }
        match line.chars().next() {
            Some('+') if !line.starts_with("+++") => {
                additions += 1;
                rendered.push_str(&format!("{}{}{}\n", COLOR_GREEN, line, COLOR_RESET));
            }
            Some('-') if !line.starts_with("---") => {
                deletions += 1;
                rendered.push_str(&format!("{}{}{}\n", COLOR_RED, line, COLOR_RESET));
            }
            _ => rendered.push_str(&format!(" {}\n", line)),
        }
    }

    let mut output = String::new();
    output.push_str(&format!(
        "{}Edited {} (+{} -{}){}\n",
        COLOR_DISTRI_HEADING, path, additions, deletions, COLOR_RESET
    ));
    output.push_str(&rendered);
    Some(output)
}

fn render_read_file(context: ToolCallStartContext<'_>) -> Option<String> {
    let path = context.input.get("path")?.as_str()?.to_string();
    let mut details = Vec::new();
    if let Some(range) = line_range_detail(context.input) {
        details.push(range);
    }
    build_simple_render(format!("Reading {}", path), details)
}

fn render_list_directory(context: ToolCallStartContext<'_>) -> Option<String> {
    let path = context.input.get("path")?.as_str()?.to_string();
    let recursive = context
        .input
        .get("recursive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut details = Vec::new();
    details.push(if recursive {
        "Recursive listing".to_string()
    } else {
        "Top level only".to_string()
    });
    build_simple_render(format!("Listing {}", path), details)
}

fn render_tree(context: ToolCallStartContext<'_>) -> Option<String> {
    let path = context.input.get("path")?.as_str()?.to_string();
    let mut details = Vec::new();
    if let Some(depth) = context.input.get("depth").and_then(|v| v.as_u64()) {
        details.push(format!("Depth {}", depth));
    }
    if context
        .input
        .get("follow_symlinks")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        details.push("Follow symlinks".to_string());
    }
    build_simple_render(format!("Tree view {}", path), details)
}

fn render_search_files(context: ToolCallStartContext<'_>) -> Option<String> {
    let path = context.input.get("path")?.as_str()?.to_string();
    let pattern = context.input.get("pattern")?.as_str()?.to_string();
    let details = vec![format!("Pattern: {}", pattern)];
    build_simple_render(format!("Searching paths under {}", path), details)
}

fn render_search_within_files(context: ToolCallStartContext<'_>) -> Option<String> {
    let path = context.input.get("path")?.as_str()?.to_string();
    let pattern = context.input.get("pattern")?.as_str()?.to_string();
    let mut details = vec![format!("Pattern: {}", pattern)];
    if let Some(depth) = context.input.get("depth").and_then(|v| v.as_u64()) {
        details.push(format!("Depth {}", depth));
    }
    if let Some(max) = context.input.get("max_results").and_then(|v| v.as_u64()) {
        details.push(format!("Max {} matches", max));
    }
    build_simple_render(format!("Searching within {}", path), details)
}

fn render_get_file_info(context: ToolCallStartContext<'_>) -> Option<String> {
    let path = context.input.get("path")?.as_str()?.to_string();
    build_simple_render(format!("Inspecting {}", path), Vec::new())
}

fn render_delete_file(context: ToolCallStartContext<'_>) -> Option<String> {
    let path = context.input.get("path")?.as_str()?.to_string();
    let recursive = context
        .input
        .get("recursive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let details = vec![if recursive {
        "Recursive delete".to_string()
    } else {
        "Single target".to_string()
    }];
    build_simple_render(format!("Deleting {}", path), details)
}

fn render_create_directory(context: ToolCallStartContext<'_>) -> Option<String> {
    let path = context.input.get("path")?.as_str()?.to_string();
    build_simple_render(format!("Creating {}", path), Vec::new())
}

fn render_copy_file(context: ToolCallStartContext<'_>) -> Option<String> {
    let source = context.input.get("source")?.as_str()?.to_string();
    let destination = context
        .input
        .get("destination")
        .and_then(|v| v.as_str())?
        .to_string();
    let details = vec![format!("-> {}", destination)];
    build_simple_render(format!("Copying {}", source), details)
}

fn render_move_file(context: ToolCallStartContext<'_>) -> Option<String> {
    let source = context.input.get("source")?.as_str()?.to_string();
    let destination = context
        .input
        .get("destination")
        .and_then(|v| v.as_str())?
        .to_string();
    let details = vec![format!("-> {}", destination)];
    build_simple_render(format!("Moving {}", source), details)
}

fn build_simple_render(title: impl AsRef<str>, details: Vec<String>) -> Option<String> {
    let mut output = String::new();
    output.push_str(&format!("{}\n", heading_line(title)));
    if let Some(line) = detail_line(&details) {
        output.push_str(&format!("   {}\n", line));
    }
    Some(output)
}

fn heading_line(text: impl AsRef<str>) -> String {
    format!("{}{}{}", COLOR_DISTRI_HEADING, text.as_ref(), COLOR_RESET)
}

fn detail_line(parts: &[String]) -> Option<String> {
    if parts.is_empty() {
        None
    } else {
        Some(format!(
            "{}{}{}",
            COLOR_GRAY,
            parts.join(" | "),
            COLOR_RESET
        ))
    }
}

fn line_range_detail(input: &Value) -> Option<String> {
    let start = input.get("start_line").and_then(|v| v.as_u64());
    let end = input.get("end_line").and_then(|v| v.as_u64());
    match (start, end) {
        (Some(s), Some(e)) => Some(format!("Lines {}-{}", s, e)),
        (Some(s), None) => Some(format!("Starting at line {}", s)),
        (None, Some(e)) => Some(format!("Through line {}", e)),
        _ => None,
    }
}

fn parse_structured_apply_diff(diff: &str) -> Option<Vec<DiffBlock>> {
    let mut blocks = Vec::new();
    let mut lines = diff.lines().peekable();

    while let Some(line) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }

        let trimmed = line.trim();
        if trimmed != "<<<<<<< SEARCH" && !trimmed.starts_with("SEARCH:") {
            warn!(
                "apply_diff renderer expected '<<<<<<< SEARCH' but found: {}",
                line
            );
            return None;
        }

        let start_line = if trimmed.starts_with("SEARCH:") {
            1usize
        } else {
            let start_line_line = lines.next()?;
            start_line_line
                .strip_prefix(":start_line:")?
                .trim()
                .parse()
                .ok()?
        };

        let expect_dash_separator = trimmed == "<<<<<<< SEARCH";
        if expect_dash_separator {
            let separator = lines.next()?;
            if separator.trim() != "-------" {
                warn!(
                    "apply_diff renderer expected '-------' separator but found: {}",
                    separator
                );
                return None;
            }
        }

        let mut search_lines = Vec::new();
        if trimmed.starts_with("SEARCH:") {
            if let Some(content) = trimmed.strip_prefix("SEARCH:") {
                search_lines.push(content.trim_start().to_string());
            }
        } else {
            while let Some(next) = lines.next() {
                if next.trim() == "=======" {
                    break;
                }
                search_lines.push(next.to_string());
            }
        }

        let mut replace_lines = Vec::new();
        if expect_dash_separator {
            while let Some(next) = lines.next() {
                if next.trim() == ">>>>>>> REPLACE" {
                    break;
                }
                replace_lines.push(next.to_string());
            }
        } else {
            while let Some(next) = lines.next() {
                if let Some(content) = next.trim().strip_prefix("REPLACE:") {
                    replace_lines.push(content.trim_start().to_string());
                    break;
                }
                replace_lines.push(next.to_string());
            }
        }

        blocks.push(DiffBlock {
            start_line,
            search: search_lines,
            replace: replace_lines,
        });
    }

    if blocks.is_empty() {
        None
    } else {
        Some(blocks)
    }
}
