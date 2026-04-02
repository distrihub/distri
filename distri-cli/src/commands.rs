use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use distri::{CreateSkillRequest, CreateSkillScriptRequest, Distri};
use tokio::fs;

use crate::config::set_client_config_value;
use crate::{
    ConfigCommands, ConnectionsCommands, PromptsCommands, SecretsCommands, SkillsCommands,
    WorkflowCommands, COLOR_BRIGHT_GREEN, COLOR_GRAY, COLOR_RESET,
};

pub fn handle_config_command(command: ConfigCommands) -> Result<()> {
    match command {
        ConfigCommands::Set { key, value } => {
            let raw_value = value
                .into_iter()
                .filter(|part| part != "=")
                .collect::<Vec<_>>()
                .join(" ");
            let path = set_client_config_value(&key, &raw_value)?;
            println!("Updated {} in {}", key, path.display());
        }
    }
    Ok(())
}

pub async fn handle_prompts_command(client: &Distri, command: PromptsCommands) -> Result<()> {
    match command {
        PromptsCommands::List => {
            println!("📋 Listing prompt templates...");
            let templates = client.list_prompt_templates().await?;
            if templates.is_empty() {
                println!("No prompt templates found.");
            } else {
                for template in templates {
                    let type_indicator = if template.is_system {
                        "system"
                    } else {
                        "custom"
                    };
                    println!(
                        "{} [{}] - {}",
                        template.name,
                        type_indicator,
                        template
                            .description
                            .as_deref()
                            .unwrap_or("(no description)")
                    );
                }
            }
        }
        PromptsCommands::Push { path } => {
            if !path.exists() {
                anyhow::bail!("Path does not exist: {}", path.display());
            }

            let mut templates = Vec::new();

            if path.is_file() {
                // Single file
                templates.push(load_template_file(&path).await?);
            } else if path.is_dir() {
                // Read all .hbs files in the directory (recursively)
                fn collect_hbs_files(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
                    for entry in std::fs::read_dir(dir)? {
                        let entry = entry?;
                        let path = entry.path();
                        if path.is_dir() {
                            collect_hbs_files(&path, files)?;
                        } else if path.is_file() {
                            if let Some(ext) = path.extension() {
                                if ext == "hbs" || ext == "handlebars" {
                                    files.push(path);
                                }
                            }
                        }
                    }
                    Ok(())
                }

                let mut files = Vec::new();
                collect_hbs_files(&path, &mut files)?;
                for file_path in files {
                    templates.push(load_template_file(&file_path).await?);
                }
            }

            if templates.is_empty() {
                println!("No .hbs template files found in {}", path.display());
                return Ok(());
            }

            println!(
                "📤 Pushing {} template(s) to {}...",
                templates.len(),
                client.base_url()
            );

            let result = client.sync_prompt_templates(&templates).await?;

            println!(
                "{}✔ Synced: {} created, {} updated{}",
                COLOR_BRIGHT_GREEN, result.created, result.updated, COLOR_RESET
            );

            // Display workspace information if configured
            if let Some(workspace_id) = client.workspace_id() {
                match client.get_workspace(workspace_id).await {
                    Ok(workspace) => {
                        let ws_type = if workspace.is_personal {
                            "Personal"
                        } else {
                            "Team"
                        };
                        println!(
                            "{}  Workspace: {} ({} - {}){}",
                            COLOR_GRAY, workspace.name, ws_type, workspace.role, COLOR_RESET
                        );
                    }
                    Err(_) => {
                        println!("{}  Workspace: {}{}", COLOR_GRAY, workspace_id, COLOR_RESET);
                    }
                }
            }

            for template in &result.templates {
                println!("  - {}", template.name);
            }
        }
    }
    Ok(())
}

pub async fn load_template_file(path: &Path) -> Result<distri::NewPromptTemplateRequest> {
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(distri::NewPromptTemplateRequest {
        name,
        template: content,
        description: None,
        version: None,
    })
}

pub async fn handle_skills_command(client: &Distri, command: SkillsCommands) -> Result<()> {
    match command {
        SkillsCommands::List => {
            println!("Listing skills...");
            let skills = client.list_skills().await?;
            if skills.is_empty() {
                println!("No skills found.");
            } else {
                for skill in skills {
                    let visibility = if skill.is_public { "public" } else { "private" };
                    let stars = if skill.star_count > 0 {
                        format!(" *{}", skill.star_count)
                    } else {
                        String::new()
                    };
                    println!(
                        "{} [{}]{} - {}",
                        skill.name,
                        visibility,
                        stars,
                        skill.description.as_deref().unwrap_or("(no description)")
                    );
                }
            }
        }
        SkillsCommands::Push { path, all } => {
            if !path.exists() {
                anyhow::bail!("Path does not exist: {}", path.display());
            }

            let mut skill_files: Vec<PathBuf> = Vec::new();

            if path.is_file() {
                skill_files.push(path.clone());
            } else if path.is_dir() {
                if !all {
                    eprintln!(
                        "Path is a directory. Re-run with --all to push all skill markdown files inside."
                    );
                    std::process::exit(1);
                }
                let mut entries = fs::read_dir(&path).await?;
                while let Some(entry) = entries.next_entry().await? {
                    let entry_path = entry.path();
                    if entry_path.is_file() {
                        if let Some(ext) = entry_path.extension() {
                            if ext == "md" {
                                skill_files.push(entry_path);
                            }
                        }
                    }
                }
            }

            if skill_files.is_empty() {
                println!("No skill markdown files found in {}", path.display());
                return Ok(());
            }

            println!(
                "Pushing {} skill(s) to {}...",
                skill_files.len(),
                client.base_url()
            );

            for skill_path in skill_files {
                let request = parse_skill_file(&skill_path).await?;
                let script_count = request.scripts.len();
                let result = client.upsert_skill(&request).await?;
                let visibility = if result.is_public {
                    "public"
                } else {
                    "private"
                };
                println!(
                    "{}  Pushed skill '{}' [{}] ({} scripts){}",
                    COLOR_BRIGHT_GREEN, result.name, visibility, script_count, COLOR_RESET
                );
            }
        }
    }
    Ok(())
}

/// TOML frontmatter for skill files.
#[derive(Debug, serde::Deserialize)]
struct SkillFrontmatter {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    is_public: bool,
}

/// Parse a skill markdown file into a CreateSkillRequest.
///
/// Format:
/// ```text
/// ---
/// name = "my-skill"
/// description = "A cool skill"
/// tags = ["foo", "bar"]
/// is_public = false
/// ---
///
/// # My Skill
/// ... content ...
///
/// ## Scripts
///
/// ### script_name
///
/// Description of the script.
///
/// ```javascript
/// // code here
/// ```
/// ```
pub async fn parse_skill_file(path: &Path) -> Result<CreateSkillRequest> {
    let raw = fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;

    // Split frontmatter and body
    let (frontmatter_str, body) = if let Some(rest) = raw.strip_prefix("---") {
        if let Some(end) = rest.find("---") {
            let fm = &rest[..end];
            let body = &rest[end + 3..];
            (fm.trim(), body.trim_start_matches('\n').to_string())
        } else {
            anyhow::bail!(
                "Invalid frontmatter in {}: missing closing ---",
                path.display()
            );
        }
    } else {
        anyhow::bail!(
            "Skill file {} must start with TOML frontmatter (---)",
            path.display()
        );
    };

    let frontmatter: SkillFrontmatter = toml::from_str(frontmatter_str)
        .with_context(|| format!("parsing frontmatter in {}", path.display()))?;

    // Extract scripts from the body
    let scripts = extract_scripts_from_markdown(&body);

    Ok(CreateSkillRequest {
        name: frontmatter.name,
        description: frontmatter.description,
        content: body,
        tags: frontmatter.tags,
        is_public: frontmatter.is_public,
        scripts,
    })
}

/// Extract scripts from markdown body.
///
/// Looks for patterns like:
/// ### script_name
/// Description text...
/// ```javascript
/// code...
/// ```
fn extract_scripts_from_markdown(body: &str) -> Vec<CreateSkillScriptRequest> {
    let mut scripts = Vec::new();
    let lines: Vec<&str> = body.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        // Look for ### heading (H3)
        if let Some(name) = lines[i].strip_prefix("### ") {
            let name = name.trim().to_string();
            i += 1;

            // Collect description lines until we hit a code fence
            let mut description_lines = Vec::new();
            while i < lines.len() && !lines[i].starts_with("```") {
                let line = lines[i].trim();
                if !line.is_empty() {
                    description_lines.push(line);
                }
                i += 1;
            }
            let description = if description_lines.is_empty() {
                None
            } else {
                Some(description_lines.join(" "))
            };

            // Parse fenced code block
            if i < lines.len() && lines[i].starts_with("```") {
                let fence_line = lines[i];
                let language = fence_line.trim_start_matches('`').trim().to_string();
                let language = if language.is_empty() {
                    "javascript".to_string()
                } else {
                    language
                };

                i += 1;
                let mut code_lines = Vec::new();
                while i < lines.len() && !lines[i].starts_with("```") {
                    code_lines.push(lines[i]);
                    i += 1;
                }
                // Skip closing fence
                if i < lines.len() {
                    i += 1;
                }

                let code = code_lines.join("\n");
                if !code.trim().is_empty() {
                    scripts.push(CreateSkillScriptRequest {
                        name,
                        description,
                        code,
                        language,
                    });
                }
            }
        } else {
            i += 1;
        }
    }

    scripts
}

pub async fn handle_connections_command(
    client: &Distri,
    command: ConnectionsCommands,
) -> Result<()> {
    match command {
        ConnectionsCommands::List => {
            let connections = client.list_connections().await?;
            if connections.is_empty() {
                println!("No connections found.");
            } else {
                for conn in connections {
                    let status = conn.status.as_deref().unwrap_or("unknown");
                    println!("{} - {} ({})", conn.id, conn.name, status);
                }
            }
        }
        ConnectionsCommands::Token { connection_id } => {
            let token = client.get_connection_token(&connection_id).await?;
            println!("{}", token.access_token);
        }
    }
    Ok(())
}

pub async fn handle_secrets_command(client: &Distri, command: SecretsCommands) -> Result<()> {
    match command {
        SecretsCommands::List => {
            let secrets = client.list_secrets().await?;
            if secrets.is_empty() {
                println!("No secrets found.");
            } else {
                for secret in secrets {
                    println!("{} = {}", secret.key, secret.masked_value);
                }
            }
        }
        SecretsCommands::Set { key, value } => {
            client
                .set_secret(&distri::NewSecretRequest {
                    key: key.clone(),
                    value,
                })
                .await?;
            println!("Secret '{}' set.", key);
        }
        SecretsCommands::Delete { key } => {
            client.delete_secret(&key).await?;
            println!("Secret '{}' deleted.", key);
        }
    }
    Ok(())
}

pub async fn push_file(client: &Distri, path: &Path) -> Result<()> {
    println!();
    println!("→ Validating configuration...");

    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;

    let definition = client.register_agent_markdown(&content).await?;

    let version = definition.version.as_deref().unwrap_or_default();
    println!(
        "{}✔ Deployed version {}{}",
        COLOR_BRIGHT_GREEN, version, COLOR_RESET
    );

    // Display workspace information if configured
    if let Some(workspace_id) = client.workspace_id() {
        // Try to fetch workspace details for a friendly display
        match client.get_workspace(workspace_id).await {
            Ok(workspace) => {
                let ws_type = if workspace.is_personal {
                    "Personal"
                } else {
                    "Team"
                };
                println!(
                    "{}  Workspace: {} ({} - {}){}",
                    COLOR_GRAY, workspace.name, ws_type, workspace.role, COLOR_RESET
                );
            }
            Err(_) => {
                // Fallback to just showing the ID if we can't fetch details
                println!("{}  Workspace: {}{}", COLOR_GRAY, workspace_id, COLOR_RESET);
            }
        }
    }

    println!();

    // Print agent URL
    let agent_url = format!("{}/agents/{}", client.base_url(), definition.name);
    println!("{}", agent_url);
    println!();

    // Print curl example
    let api_key_header = if client.has_auth() {
        "\n  -H \"Authorization: Bearer $DISTRI_API_KEY\" \\"
    } else {
        ""
    };

    println!("{}# Example curl command:{}", COLOR_GRAY, COLOR_RESET);
    println!(
        r#"{}curl -X POST "{}" \
  -H "Content-Type: application/json" \{}
  -d '{{"message": {{"role": "user", "parts": [{{"type": "text", "text": "Hello"}}]}}}}'
{}"#,
        COLOR_GRAY, agent_url, api_key_header, COLOR_RESET
    );

    Ok(())
}

pub async fn handle_workflow_command(
    client: &distri::Distri,
    command: WorkflowCommands,
) -> Result<()> {
    use distri::workflow::*;

    match command {
        WorkflowCommands::Run {
            workflow: workflow_ref,
            step,
            input,
            entry,
        } => {
            // Resolve workflow: local file or server name/id
            let looks_like_path = workflow_ref.contains('/')
                || workflow_ref.contains('\\')
                || workflow_ref.ends_with(".json");

            let mut workflow = if std::path::Path::new(&workflow_ref).exists() {
                let content = fs::read_to_string(&workflow_ref)
                    .await
                    .with_context(|| format!("Failed to read workflow file: {}", workflow_ref))?;
                serde_json::from_str::<WorkflowDefinition>(&content)
                    .with_context(|| "Failed to parse workflow JSON")?
            } else if looks_like_path {
                anyhow::bail!(
                    "File not found: {}\n  Check the path and try again.",
                    workflow_ref
                );
            } else {
                println!("  Fetching workflow '{}' from server...", workflow_ref);
                let list = client
                    .list_workflows()
                    .await
                    .with_context(|| "Failed to list workflows from server")?;
                let record = list
                    .workflows
                    .iter()
                    .find(|w| w.name == workflow_ref || w.id == workflow_ref)
                    .ok_or_else(|| {
                        anyhow::anyhow!("Workflow '{}' not found on server", workflow_ref)
                    })?;
                let full = client
                    .get_workflow(&record.id)
                    .await
                    .with_context(|| "Failed to fetch workflow")?;
                serde_json::from_value::<WorkflowDefinition>(full.definition)
                    .with_context(|| "Failed to parse workflow definition from server")?
            };

            // Apply entry point if specified (before input, since it may set preset results)
            if let Some(ref entry_id) = entry {
                workflow = workflow
                    .apply_entry_point(entry_id)
                    .map_err(|e| anyhow::anyhow!(e))?;
                println!("  Using entry point: {}", entry_id);
            }

            // Apply input if provided
            if let Some(ref input_json) = input {
                let input_val: serde_json::Value = serde_json::from_str(input_json)
                    .with_context(|| "Failed to parse --input JSON")?;
                workflow = workflow
                    .with_input(input_val)
                    .map_err(|e| anyhow::anyhow!(e))?;
            }

            println!(
                "{}→ Workflow:{} {} ({})",
                COLOR_BRIGHT_GREEN,
                COLOR_RESET,
                workflow.id,
                workflow.steps.len()
            );
            println!(
                "  {} steps, status: {:?}",
                workflow.steps.len(),
                workflow.status
            );
            println!();

            // Run with event streaming
            let arc_client = Arc::new(client.clone());
            let mut session = distri::WorkflowSession::new(arc_client, workflow);
            let mut rx = session.take_events().unwrap();

            if step {
                // Step mode: print events as they come, pause between steps
                let handle = tokio::spawn(async move { session.run().await });
                let mut last_step = String::new();
                while let Some(event) = rx.recv().await {
                    match &event {
                        WorkflowEvent::StepStarted {
                            step_id,
                            step_label,
                            ..
                        } => {
                            if !last_step.is_empty() {
                                print!("  Press Enter for next step (q to quit): ");
                                io::stdout().flush()?;
                                let mut buf = String::new();
                                io::stdin().read_line(&mut buf)?;
                                if buf.trim() == "q" {
                                    break;
                                }
                            }
                            println!("  ⏳ {} — {}", step_id, step_label);
                            last_step = step_id.clone();
                        }
                        WorkflowEvent::StepCompleted { step_id, .. } => {
                            println!("  ✅ {}", step_id);
                        }
                        WorkflowEvent::StepFailed { step_id, error, .. } => {
                            println!("  ❌ {} — {}", step_id, error);
                        }
                        WorkflowEvent::WorkflowCompleted {
                            status,
                            steps_done,
                            steps_failed,
                            ..
                        } => {
                            println!(
                                "\n  Status: {:?} ({} done, {} failed)",
                                status, steps_done, steps_failed
                            );
                        }
                        _ => {}
                    }
                }
                let _ = handle.await;
            } else {
                // Run all, print events as they stream
                let handle = tokio::spawn(async move { session.run().await });
                while let Some(event) = rx.recv().await {
                    match &event {
                        WorkflowEvent::WorkflowStarted { total_steps, .. } => {
                            println!("  Starting workflow ({} steps)", total_steps);
                        }
                        WorkflowEvent::StepStarted {
                            step_id,
                            step_label,
                            ..
                        } => {
                            print!("  ⏳ {} — {}...", step_id, step_label);
                            io::stdout().flush()?;
                        }
                        WorkflowEvent::StepCompleted { step_id: _, .. } => {
                            println!(" ✅");
                        }
                        WorkflowEvent::StepFailed {
                            step_id: _, error, ..
                        } => {
                            println!(" ❌ {}", error);
                        }
                        WorkflowEvent::WorkflowCompleted {
                            status,
                            steps_done,
                            steps_failed,
                            ..
                        } => {
                            println!(
                                "\n  Status: {:?} ({} done, {} failed)",
                                status, steps_done, steps_failed
                            );
                        }
                        WorkflowEvent::StepWaiting {
                            step_id, message, ..
                        } => {
                            println!(" ✋ {} — waiting for input: {}", step_id, message);
                        }
                    }
                }
                let _ = handle.await;
            }
        }

        WorkflowCommands::Status { path } => {
            let content = fs::read_to_string(&path)
                .await
                .with_context(|| format!("Failed to read: {}", path.display()))?;
            let workflow: WorkflowDefinition =
                serde_json::from_str(&content).with_context(|| "Failed to parse workflow JSON")?;

            println!(
                "{}Workflow:{} {}",
                COLOR_BRIGHT_GREEN, COLOR_RESET, workflow.id
            );
            println!("  Status: {:?}", workflow.status);
            println!(
                "  Steps: {}/{}",
                workflow
                    .steps
                    .iter()
                    .filter(|s| s.status == StepStatus::Done)
                    .count(),
                workflow.steps.len()
            );
            for (i, s) in workflow.steps.iter().enumerate() {
                let icon = match s.status {
                    StepStatus::Done => "✅",
                    StepStatus::Failed => "❌",
                    StepStatus::Running => "⏳",
                    StepStatus::Blocked => "🚫",
                    _ => "⬜",
                };
                println!("  {} {}. {}", icon, i + 1, s.label);
            }
        }

        WorkflowCommands::Push { path, name } => {
            let content = fs::read_to_string(&path)
                .await
                .with_context(|| format!("Failed to read workflow file: {}", path.display()))?;
            let definition: serde_json::Value =
                serde_json::from_str(&content).with_context(|| "Failed to parse workflow JSON")?;

            let wf_name = name.unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or("workflow".to_string())
            });

            match client.push_workflow(&wf_name, definition).await {
                Ok(record) => {
                    println!(
                        "{}→ Pushed:{} {} ({})",
                        COLOR_BRIGHT_GREEN, COLOR_RESET, record.name, record.id
                    );
                }
                Err(e) => {
                    println!("Failed to push workflow: {}", e);
                }
            }
        }

        WorkflowCommands::List => match client.list_workflows().await {
            Ok(response) => {
                if response.workflows.is_empty() {
                    println!("No workflows found.");
                } else {
                    for w in &response.workflows {
                        let tpl = if w.is_template { " [template]" } else { "" };
                        println!("  {} ({} steps){}", w.name, w.step_count, tpl);
                    }
                    println!("\n  {} total", response.total);
                }
            }
            Err(e) => println!("Failed to list workflows: {}", e),
        },
    }
    Ok(())
}
