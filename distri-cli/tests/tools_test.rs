//! Integration tests for local CLI tools (Bash, Read, Write, Edit, Glob, Grep).

use distri::ExternalToolRegistry;
use distri_types::{AgentEvent, AgentEventType, ToolCall};
use serde_json::json;
use std::path::Path;
use tempfile::TempDir;

// Pull in the tool registration
// We can't import private modules directly, so we test through the registry
fn make_event() -> AgentEvent {
    AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: "test-thread".to_string(),
        run_id: "test-run".to_string(),
        task_id: "test-task".to_string(),
        agent_id: "test-agent".to_string(),
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
        event: AgentEventType::RunFinished {
            success: true,
            total_steps: 0,
            failed_steps: 0,
            usage: None,
        },
    }
}

fn make_call(tool_name: &str, input: serde_json::Value) -> ToolCall {
    ToolCall {
        tool_call_id: "tc-1".to_string(),
        tool_name: tool_name.to_string(),
        input,
    }
}

/// Helper: register all tools and return (registry, temp_dir)
fn setup() -> (ExternalToolRegistry, TempDir) {
    let dir = TempDir::new().unwrap();
    let registry = ExternalToolRegistry::new();
    // We need to register tools the same way main.rs does.
    // Since tools/mod.rs::register_all is pub, we can call it if we
    // add distri-cli as a dependency — but that's circular.
    // Instead, test through individual registrations using the same patterns.
    register_all_tools(&registry, "test-agent", dir.path());
    (registry, dir)
}

/// Replicate what distri-cli::tools::register_all does for test purposes.
fn register_all_tools(registry: &ExternalToolRegistry, agent_id: &str, workspace: &Path) {
    register_bash(registry, agent_id, workspace);
    register_read(registry, agent_id, workspace);
    register_write(registry, agent_id, workspace);
    register_edit(registry, agent_id, workspace);
    register_glob(registry, agent_id, workspace);
    register_grep(registry, agent_id, workspace);
}

fn register_bash(registry: &ExternalToolRegistry, agent_id: &str, workspace: &Path) {
    let workspace = workspace.to_path_buf();
    registry.register(
        agent_id.to_string(),
        "Bash".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let input: serde_json::Value = call.input;
                let command = input["command"].as_str().unwrap_or("");
                let mut cmd = tokio::process::Command::new("bash");
                cmd.arg("-lc").arg(command).current_dir(&workspace);
                let output = cmd.output().await?;
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);
                Ok(distri_types::ToolResponse::direct(
                    call.tool_call_id,
                    "Bash".to_string(),
                    json!({ "stdout": stdout, "stderr": stderr, "exit_code": exit_code }),
                ))
            }
        },
    );
}

fn register_read(registry: &ExternalToolRegistry, agent_id: &str, workspace: &Path) {
    let workspace = workspace.to_path_buf();
    registry.register(
        agent_id.to_string(),
        "Read".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let input: serde_json::Value = call.input;
                let file_path = input["file_path"].as_str().unwrap_or("");
                let path = if Path::new(file_path).is_absolute() {
                    std::path::PathBuf::from(file_path)
                } else {
                    workspace.join(file_path)
                };
                let content = tokio::fs::read_to_string(&path).await?;
                let lines: Vec<&str> = content.lines().collect();
                let offset = input["offset"].as_u64().unwrap_or(0) as usize;
                let limit = input["limit"].as_u64().unwrap_or(2000) as usize;
                let total = lines.len();
                let start = offset.min(total);
                let end = (start + limit).min(total);
                let numbered: String = lines[start..end]
                    .iter()
                    .enumerate()
                    .map(|(i, l)| format!("{:>4}\t{}", start + i + 1, l))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(distri_types::ToolResponse::direct(
                    call.tool_call_id,
                    "Read".to_string(),
                    json!({
                        "content": numbered,
                        "file_path": file_path,
                        "total_lines": total,
                        "lines_read": end - start,
                        "truncated": end < total,
                    }),
                ))
            }
        },
    );
}

fn register_write(registry: &ExternalToolRegistry, agent_id: &str, workspace: &Path) {
    let workspace = workspace.to_path_buf();
    registry.register(
        agent_id.to_string(),
        "Write".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let input: serde_json::Value = call.input;
                let file_path = input["file_path"].as_str().unwrap_or("");
                let content = input["content"].as_str().unwrap_or("");
                let path = if Path::new(file_path).is_absolute() {
                    std::path::PathBuf::from(file_path)
                } else {
                    workspace.join(file_path)
                };
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&path, content).await?;
                Ok(distri_types::ToolResponse::direct(
                    call.tool_call_id,
                    "Write".to_string(),
                    json!({ "file_path": file_path, "bytes_written": content.len(), "success": true }),
                ))
            }
        },
    );
}

fn register_edit(registry: &ExternalToolRegistry, agent_id: &str, workspace: &Path) {
    let workspace = workspace.to_path_buf();
    registry.register(
        agent_id.to_string(),
        "Edit".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let input: serde_json::Value = call.input;
                let file_path = input["file_path"].as_str().unwrap_or("");
                let old_string = input["old_string"].as_str().unwrap_or("");
                let new_string = input["new_string"].as_str().unwrap_or("");
                let replace_all = input["replace_all"].as_bool().unwrap_or(false);
                let path = if Path::new(file_path).is_absolute() {
                    std::path::PathBuf::from(file_path)
                } else {
                    workspace.join(file_path)
                };
                let content = tokio::fs::read_to_string(&path).await?;
                let count = content.matches(old_string).count();
                if count == 0 {
                    anyhow::bail!("old_string not found");
                }
                if count > 1 && !replace_all {
                    anyhow::bail!("old_string found {} times, use replace_all", count);
                }
                let new_content = if replace_all {
                    content.replace(old_string, new_string)
                } else {
                    content.replacen(old_string, new_string, 1)
                };
                tokio::fs::write(&path, &new_content).await?;
                Ok(distri_types::ToolResponse::direct(
                    call.tool_call_id,
                    "Edit".to_string(),
                    json!({ "file_path": file_path, "replacements": if replace_all { count } else { 1 }, "success": true }),
                ))
            }
        },
    );
}

fn register_glob(registry: &ExternalToolRegistry, agent_id: &str, workspace: &Path) {
    let workspace = workspace.to_path_buf();
    registry.register(
        agent_id.to_string(),
        "Glob".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let input: serde_json::Value = call.input;
                let pattern = input["pattern"].as_str().unwrap_or("");
                let full_pattern = format!("{}/{}", workspace.display(), pattern);
                let entries: Vec<_> = glob::glob(&full_pattern)
                    .map_err(|e| anyhow::anyhow!("{}", e))?
                    .filter_map(|e| e.ok())
                    .take(100)
                    .collect();
                let filenames: Vec<String> = entries
                    .iter()
                    .filter_map(|p| p.strip_prefix(&workspace).ok().map(|r| r.to_string_lossy().to_string()))
                    .collect();
                Ok(distri_types::ToolResponse::direct(
                    call.tool_call_id,
                    "Glob".to_string(),
                    json!({ "filenames": filenames, "num_files": filenames.len(), "truncated": false }),
                ))
            }
        },
    );
}

fn register_grep(registry: &ExternalToolRegistry, agent_id: &str, workspace: &Path) {
    let workspace = workspace.to_path_buf();
    registry.register(
        agent_id.to_string(),
        "Grep".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let input: serde_json::Value = call.input;
                let pattern = input["pattern"].as_str().unwrap_or("");
                let output = tokio::process::Command::new("rg")
                    .args(["--files-with-matches", "--", pattern])
                    .arg(workspace.to_string_lossy().as_ref())
                    .output()
                    .await?;
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                Ok(distri_types::ToolResponse::direct(
                    call.tool_call_id,
                    "Grep".to_string(),
                    json!({ "output": stdout, "total_lines": stdout.lines().count(), "truncated": false, "exit_code": output.status.code() }),
                ))
            }
        },
    );
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_bash_echo() {
    let (registry, _dir) = setup();
    let call = make_call("Bash", json!({"command": "echo hello"}));
    let result = registry
        .try_handle("test-agent", "Bash", &call, &make_event())
        .await;
    let resp = result.unwrap().unwrap();
    let data = &resp.parts[0];
    let value = match data {
        distri_types::Part::Data(v) => v,
        _ => panic!("expected Data part"),
    };
    assert_eq!(value["stdout"].as_str().unwrap().trim(), "hello");
    assert_eq!(value["exit_code"].as_i64().unwrap(), 0);
}

#[tokio::test]
async fn test_bash_exit_code() {
    let (registry, _dir) = setup();
    let call = make_call("Bash", json!({"command": "exit 42"}));
    let result = registry
        .try_handle("test-agent", "Bash", &call, &make_event())
        .await;
    let resp = result.unwrap().unwrap();
    let value = match &resp.parts[0] {
        distri_types::Part::Data(v) => v,
        _ => panic!("expected Data part"),
    };
    assert_eq!(value["exit_code"].as_i64().unwrap(), 42);
}

#[tokio::test]
async fn test_write_and_read() {
    let (registry, _dir) = setup();

    // Write a file
    let call = make_call(
        "Write",
        json!({
            "file_path": "test.txt",
            "content": "line one\nline two\nline three"
        }),
    );
    let resp = registry
        .try_handle("test-agent", "Write", &call, &make_event())
        .await
        .unwrap()
        .unwrap();
    let value = match &resp.parts[0] {
        distri_types::Part::Data(v) => v,
        _ => panic!("expected Data part"),
    };
    assert_eq!(value["success"].as_bool().unwrap(), true);
    assert_eq!(value["bytes_written"].as_u64().unwrap(), 28);

    // Read it back
    let call = make_call("Read", json!({"file_path": "test.txt"}));
    let resp = registry
        .try_handle("test-agent", "Read", &call, &make_event())
        .await
        .unwrap()
        .unwrap();
    let value = match &resp.parts[0] {
        distri_types::Part::Data(v) => v,
        _ => panic!("expected Data part"),
    };
    assert_eq!(value["total_lines"].as_u64().unwrap(), 3);
    assert_eq!(value["lines_read"].as_u64().unwrap(), 3);
    assert_eq!(value["truncated"].as_bool().unwrap(), false);
    let content = value["content"].as_str().unwrap();
    assert!(content.contains("line one"));
    assert!(content.contains("line three"));
}

#[tokio::test]
async fn test_read_with_offset_and_limit() {
    let (registry, dir) = setup();
    // Create a file with 10 lines
    let content: String = (1..=10)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.path().join("big.txt"), &content).unwrap();

    let call = make_call(
        "Read",
        json!({"file_path": "big.txt", "offset": 2, "limit": 3}),
    );
    let resp = registry
        .try_handle("test-agent", "Read", &call, &make_event())
        .await
        .unwrap()
        .unwrap();
    let value = match &resp.parts[0] {
        distri_types::Part::Data(v) => v,
        _ => panic!("expected Data part"),
    };
    assert_eq!(value["total_lines"].as_u64().unwrap(), 10);
    assert_eq!(value["lines_read"].as_u64().unwrap(), 3);
    assert_eq!(value["truncated"].as_bool().unwrap(), true);
    let content = value["content"].as_str().unwrap();
    assert!(content.contains("line 3")); // offset=2 means start at line index 2 (line 3)
    assert!(content.contains("line 5")); // 3 lines: 3, 4, 5
    assert!(!content.contains("line 6"));
}

#[tokio::test]
async fn test_write_creates_parent_dirs() {
    let (registry, _dir) = setup();
    let call = make_call(
        "Write",
        json!({
            "file_path": "deep/nested/dir/file.txt",
            "content": "hello"
        }),
    );
    let resp = registry
        .try_handle("test-agent", "Write", &call, &make_event())
        .await
        .unwrap()
        .unwrap();
    let value = match &resp.parts[0] {
        distri_types::Part::Data(v) => v,
        _ => panic!("expected Data part"),
    };
    assert_eq!(value["success"].as_bool().unwrap(), true);
}

#[tokio::test]
async fn test_edit_single_replacement() {
    let (registry, dir) = setup();
    std::fs::write(dir.path().join("edit.txt"), "hello world").unwrap();

    let call = make_call(
        "Edit",
        json!({
            "file_path": "edit.txt",
            "old_string": "world",
            "new_string": "rust"
        }),
    );
    let resp = registry
        .try_handle("test-agent", "Edit", &call, &make_event())
        .await
        .unwrap()
        .unwrap();
    let value = match &resp.parts[0] {
        distri_types::Part::Data(v) => v,
        _ => panic!("expected Data part"),
    };
    assert_eq!(value["replacements"].as_u64().unwrap(), 1);

    let content = std::fs::read_to_string(dir.path().join("edit.txt")).unwrap();
    assert_eq!(content, "hello rust");
}

#[tokio::test]
async fn test_edit_replace_all() {
    let (registry, dir) = setup();
    std::fs::write(dir.path().join("multi.txt"), "foo bar foo baz foo").unwrap();

    let call = make_call(
        "Edit",
        json!({
            "file_path": "multi.txt",
            "old_string": "foo",
            "new_string": "qux",
            "replace_all": true
        }),
    );
    let resp = registry
        .try_handle("test-agent", "Edit", &call, &make_event())
        .await
        .unwrap()
        .unwrap();
    let value = match &resp.parts[0] {
        distri_types::Part::Data(v) => v,
        _ => panic!("expected Data part"),
    };
    assert_eq!(value["replacements"].as_u64().unwrap(), 3);

    let content = std::fs::read_to_string(dir.path().join("multi.txt")).unwrap();
    assert_eq!(content, "qux bar qux baz qux");
}

#[tokio::test]
async fn test_edit_fails_on_ambiguous_match() {
    let (registry, dir) = setup();
    std::fs::write(dir.path().join("ambig.txt"), "aaa aaa aaa").unwrap();

    let call = make_call(
        "Edit",
        json!({
            "file_path": "ambig.txt",
            "old_string": "aaa",
            "new_string": "bbb"
        }),
    );
    let result = registry
        .try_handle("test-agent", "Edit", &call, &make_event())
        .await
        .unwrap();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("found 3 times"));
}

#[tokio::test]
async fn test_edit_fails_on_not_found() {
    let (registry, dir) = setup();
    std::fs::write(dir.path().join("nf.txt"), "hello").unwrap();

    let call = make_call(
        "Edit",
        json!({
            "file_path": "nf.txt",
            "old_string": "nonexistent",
            "new_string": "x"
        }),
    );
    let result = registry
        .try_handle("test-agent", "Edit", &call, &make_event())
        .await
        .unwrap();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[tokio::test]
async fn test_glob_finds_files() {
    let (registry, dir) = setup();
    std::fs::write(dir.path().join("a.rs"), "").unwrap();
    std::fs::write(dir.path().join("b.rs"), "").unwrap();
    std::fs::write(dir.path().join("c.txt"), "").unwrap();

    let call = make_call("Glob", json!({"pattern": "*.rs"}));
    let resp = registry
        .try_handle("test-agent", "Glob", &call, &make_event())
        .await
        .unwrap()
        .unwrap();
    let value = match &resp.parts[0] {
        distri_types::Part::Data(v) => v,
        _ => panic!("expected Data part"),
    };
    assert_eq!(value["num_files"].as_u64().unwrap(), 2);
    let filenames = value["filenames"].as_array().unwrap();
    let names: Vec<&str> = filenames.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(names.contains(&"a.rs"));
    assert!(names.contains(&"b.rs"));
    assert!(!names.iter().any(|n| n.ends_with(".txt")));
}

#[tokio::test]
async fn test_glob_nested() {
    let (registry, dir) = setup();
    std::fs::create_dir_all(dir.path().join("src/utils")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "").unwrap();
    std::fs::write(dir.path().join("src/utils/helper.rs"), "").unwrap();

    let call = make_call("Glob", json!({"pattern": "**/*.rs"}));
    let resp = registry
        .try_handle("test-agent", "Glob", &call, &make_event())
        .await
        .unwrap()
        .unwrap();
    let value = match &resp.parts[0] {
        distri_types::Part::Data(v) => v,
        _ => panic!("expected Data part"),
    };
    assert_eq!(value["num_files"].as_u64().unwrap(), 2);
}

#[tokio::test]
async fn test_grep_finds_content() {
    let (registry, dir) = setup();
    std::fs::write(
        dir.path().join("search.txt"),
        "hello world\nfoo bar\nhello again",
    )
    .unwrap();
    std::fs::write(dir.path().join("other.txt"), "no match here").unwrap();

    let call = make_call("Grep", json!({"pattern": "hello"}));
    let resp = registry
        .try_handle("test-agent", "Grep", &call, &make_event())
        .await
        .unwrap()
        .unwrap();
    let value = match &resp.parts[0] {
        distri_types::Part::Data(v) => v,
        _ => panic!("expected Data part"),
    };
    let output = value["output"].as_str().unwrap();
    assert!(output.contains("search.txt"));
    assert!(!output.contains("other.txt"));
}

#[tokio::test]
async fn test_grep_no_matches() {
    let (registry, dir) = setup();
    std::fs::write(dir.path().join("empty.txt"), "nothing here").unwrap();

    let call = make_call("Grep", json!({"pattern": "zzzzzzz"}));
    let resp = registry
        .try_handle("test-agent", "Grep", &call, &make_event())
        .await
        .unwrap()
        .unwrap();
    let value = match &resp.parts[0] {
        distri_types::Part::Data(v) => v,
        _ => panic!("expected Data part"),
    };
    assert_eq!(value["total_lines"].as_u64().unwrap(), 0);
}

// Also add formatter tests for tool call formatting
#[cfg(test)]
mod formatter_tests {
    use distri_formatter::state::format_tool_call;
    use serde_json::json;

    #[test]
    fn test_format_bash() {
        let result = format_tool_call("Bash", &json!({"command": "echo hello"}));
        assert_eq!(result, "Bash(\"echo hello\")");
    }

    #[test]
    fn test_format_bash_multiline_shows_first_line() {
        let result = format_tool_call("Bash", &json!({"command": "echo hello\necho world"}));
        assert_eq!(result, "Bash(\"echo hello\")");
    }

    #[test]
    fn test_format_read() {
        let result = format_tool_call("Read", &json!({"file_path": "src/main.rs"}));
        assert_eq!(result, "Read(\"src/main.rs\")");
    }

    #[test]
    fn test_format_read_with_offset() {
        let result = format_tool_call(
            "Read",
            &json!({"file_path": "big.rs", "offset": 99, "limit": 50}),
        );
        assert_eq!(result, "Read(\"big.rs\", lines 100-149)");
    }

    #[test]
    fn test_format_write() {
        let result = format_tool_call(
            "Write",
            &json!({"file_path": "out.txt", "content": "a\nb\nc\n"}),
        );
        assert_eq!(result, "Write(\"out.txt\", 3 lines)");
    }

    #[test]
    fn test_format_edit() {
        let result = format_tool_call(
            "Edit",
            &json!({"file_path": "f.rs", "old_string": "a", "new_string": "b"}),
        );
        assert_eq!(result, "Edit(\"f.rs\")");
    }

    #[test]
    fn test_format_edit_replace_all() {
        let result = format_tool_call(
            "Edit",
            &json!({"file_path": "f.rs", "old_string": "a", "new_string": "b", "replace_all": true}),
        );
        assert_eq!(result, "Edit(\"f.rs\", replace_all)");
    }

    #[test]
    fn test_format_glob() {
        let result = format_tool_call("Glob", &json!({"pattern": "**/*.rs"}));
        assert_eq!(result, "Glob(\"**/*.rs\")");
    }

    #[test]
    fn test_format_glob_with_path() {
        let result = format_tool_call(
            "Glob",
            &json!({"pattern": "*.ts", "path": "src/components"}),
        );
        assert_eq!(result, "Glob(\"*.ts\", path: \"src/components\")");
    }

    #[test]
    fn test_format_grep() {
        let result = format_tool_call("Grep", &json!({"pattern": "TODO"}));
        assert_eq!(result, "Grep(\"TODO\")");
    }

    #[test]
    fn test_format_grep_with_path() {
        let result = format_tool_call("Grep", &json!({"pattern": "import", "path": "src/"}));
        assert_eq!(result, "Grep(\"import\", path: \"src/\")");
    }
}
