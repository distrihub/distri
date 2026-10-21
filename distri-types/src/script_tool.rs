//! The script-tool contract shared by every host that runs TypeScript tools under Deno:
//! the `distri` CLI (`.distri/tools/*.ts`) and Distri Builder (`apps/<app>/scripts/*.ts`).
//!
//! The TypeScript half lives in `distri-tool/` (`@distri/tool`): `defineTool`, the
//! stdin/stdout run harness and the extractor. This module is the Rust half: the shape of
//! `export const tool = defineTool({...})` and the parser for the extractor's output line.
//!
//! The contract (frozen): input is one JSON document on stdin, output is one JSON document on
//! stdout, logs go to stderr, a non-zero exit is a failed call.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ExternalTool, ToolDefinition};

/// The prefix of the line `distri-tool/extract.ts` prints: `__distri_tool__ {"tool": ...}`.
pub const EXTRACT_SENTINEL: &str = "__distri_tool__ ";

/// The `export const tool = defineTool({...})` shape. Same fields as [`ExternalTool`] minus the
/// engine-only ones, so a script with a `tool` export is an external tool for cloud agents and a
/// builtin tool for local ones.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ToolExport {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// JSON Schema of the input document.
    #[serde(default = "default_parameters")]
    pub parameters: serde_json::Value,
}

fn default_parameters() -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": {}})
}

impl From<ToolExport> for ExternalTool {
    fn from(t: ToolExport) -> Self {
        ExternalTool {
            name: t.name,
            description: t.description,
            parameters: t.parameters,
            is_final: false,
            prompt: None,
        }
    }
}

impl From<ToolExport> for ToolDefinition {
    fn from(t: ToolExport) -> Self {
        ToolDefinition {
            name: t.name,
            description: t.description,
            parameters: t.parameters,
            examples: None,
            output_schema: None,
            prompt: None,
        }
    }
}

/// What follows [`EXTRACT_SENTINEL`] on the extractor's result line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractOutput {
    pub tool: Option<ToolExport>,
}

/// Why the extractor's output could not be read as a tool export.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExtractError {
    #[error("tool export: {0}")]
    Invalid(String),
    #[error("tool export: `name` must not be empty")]
    EmptyName,
}

/// The tool a script exports, from the extractor's stdout. The result is the last line carrying
/// [`EXTRACT_SENTINEL`]; anything else on stdout is the script's own import-time output and is
/// ignored. Exit 0 without a sentinel line (the script exited on its own while being imported)
/// is "no tool", not an error.
pub fn parse_extract_stdout(stdout: &[u8]) -> Result<Option<ToolExport>, ExtractError> {
    let text = String::from_utf8_lossy(stdout);
    let Some(line) = text
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix(EXTRACT_SENTINEL))
    else {
        return Ok(None);
    };
    let out: ExtractOutput =
        serde_json::from_str(line.trim()).map_err(|e| ExtractError::Invalid(e.to_string()))?;
    match out.tool {
        Some(t) if t.name.trim().is_empty() => Err(ExtractError::EmptyName),
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_last_sentinel_line_and_ignores_script_output() {
        let stdout = b"hello from import\n__distri_tool__ {\"tool\": {\"name\": \"old\"}}\n\n__distri_tool__ {\"tool\": {\"name\": \"lookup_order\", \"description\": \"d\"}}\n";
        let tool = parse_extract_stdout(stdout).unwrap().unwrap();
        assert_eq!(tool.name, "lookup_order");
        assert_eq!(tool.description, "d");
        assert_eq!(tool.parameters, default_parameters());
    }

    #[test]
    fn no_sentinel_or_null_tool_is_no_tool() {
        assert_eq!(parse_extract_stdout(b"just a job\n").unwrap(), None);
        assert_eq!(
            parse_extract_stdout(b"\n__distri_tool__ {\"tool\": null}\n").unwrap(),
            None
        );
    }

    #[test]
    fn bad_exports_are_errors() {
        assert_eq!(
            parse_extract_stdout(b"__distri_tool__ {\"tool\": {\"name\": \" \"}}").unwrap_err(),
            ExtractError::EmptyName
        );
        assert!(matches!(
            parse_extract_stdout(b"__distri_tool__ {\"tool\": 5}").unwrap_err(),
            ExtractError::Invalid(_)
        ));
        assert!(matches!(
            parse_extract_stdout(b"__distri_tool__ not json").unwrap_err(),
            ExtractError::Invalid(_)
        ));
    }

    #[test]
    fn converts_to_the_engine_shapes() {
        let t = ToolExport {
            name: "x".into(),
            description: "y".into(),
            parameters: default_parameters(),
        };
        let def: ToolDefinition = t.clone().into();
        assert_eq!(def.name, "x");
        let ext: ExternalTool = t.into();
        assert!(!ext.is_final);
    }
}
