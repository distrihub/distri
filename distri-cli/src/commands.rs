use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use distri::{CreateSkillRequest, Distri};
use tokio::fs;

use crate::{
    ConnectionsCommands, ProfileCommands, ProfileConfigCommands, PromptsCommands, SecretsCommands,
    SkillsCommands, COLOR_BRIGHT_GREEN, COLOR_GRAY, COLOR_RESET,
};

fn mask_api_key(key: &str) -> String {
    if key.len() > 14 {
        format!("{}...{}", &key[..10], &key[key.len() - 4..])
    } else {
        "***".to_string()
    }
}

pub fn handle_profile_command(command: ProfileCommands) -> Result<()> {
    use crate::credentials::{
        delete_profile, get_active_profile, list_profiles, load_profile, save_profile,
        set_active_profile, unset_profile_keys, ProfileValues,
    };

    match command {
        ProfileCommands::List => {
            let active = get_active_profile();
            let profiles = list_profiles()?;
            if profiles.is_empty() {
                println!("No profiles found. Run `distri login` or `distri profile config set` to create one.");
                return Ok(());
            }
            for (name, values) in &profiles {
                let marker = if name == &active { "*" } else { " " };
                let key_str = values
                    .api_key
                    .as_deref()
                    .map(mask_api_key)
                    .unwrap_or_else(|| "(none)".to_string());
                let ws_str = values.workspace_id.as_deref().unwrap_or("(none)");
                let url_str = values
                    .api_url
                    .as_deref()
                    .unwrap_or("https://api.distri.dev/v1");
                println!(
                    "{} {:<12}  api_key={:<20}  workspace={:<36}  url={}",
                    marker, name, key_str, ws_str, url_str
                );
            }
        }

        ProfileCommands::Use { name } => {
            let profiles = list_profiles()?;
            let exists = profiles.iter().any(|(n, _)| n == &name);
            if !exists {
                anyhow::bail!(
                    "Profile '{}' not found. Run `distri profile list` to see available profiles.",
                    name
                );
            }
            set_active_profile(&name)?;
            println!("Active profile set to '{}'.", name);
        }

        ProfileCommands::Show { name } => {
            let profile_name = name.unwrap_or_else(get_active_profile);
            match load_profile(&profile_name)? {
                None => {
                    anyhow::bail!(
                        "Profile '{}' not found. Run `distri profile list` to see available profiles.",
                        profile_name
                    );
                }
                Some(values) => {
                    println!("Profile: {}", profile_name);
                    println!(
                        "  api_key      = {}",
                        values
                            .api_key
                            .as_deref()
                            .map(mask_api_key)
                            .unwrap_or_else(|| "(not set)".to_string())
                    );
                    println!(
                        "  workspace_id = {}",
                        values.workspace_id.as_deref().unwrap_or("(not set)")
                    );
                    println!(
                        "  api_url      = {}",
                        values
                            .api_url
                            .as_deref()
                            .unwrap_or("https://api.distri.dev/v1 (default)")
                    );
                }
            }
        }

        ProfileCommands::Delete { name, yes } => {
            let active = get_active_profile();
            if name == active {
                anyhow::bail!(
                    "Cannot delete the active profile '{}'. Run `distri profile use <other>` first.",
                    name
                );
            }
            if !yes {
                print!("Delete profile '{}'? [y/N] ", name);
                std::io::stdout().flush().ok();
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).ok();
                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Aborted.");
                    return Ok(());
                }
            }
            delete_profile(&name)?;
            println!("Profile '{}' deleted.", name);
        }

        ProfileCommands::Config { command } => match command {
            ProfileConfigCommands::Set {
                profile,
                api_key,
                workspace_id,
                api_url,
            } => {
                if api_key.is_none() && workspace_id.is_none() && api_url.is_none() {
                    anyhow::bail!(
                        "At least one of --api-key, --workspace-id, or --api-url is required."
                    );
                }
                if let Some(ref ws) = workspace_id {
                    uuid::Uuid::parse_str(ws).with_context(|| {
                        format!("Invalid workspace_id: '{}' is not a valid UUID", ws)
                    })?;
                }
                let profile_name = profile.unwrap_or_else(get_active_profile);
                let values = ProfileValues {
                    api_key,
                    workspace_id,
                    api_url,
                };
                save_profile(&profile_name, &values)?;
                println!("Updated profile '{}'.", profile_name);
            }

            ProfileConfigCommands::Unset {
                profile,
                api_key,
                workspace_id,
                api_url,
            } => {
                if !api_key && !workspace_id && !api_url {
                    anyhow::bail!(
                        "At least one of --api-key, --workspace-id, or --api-url is required."
                    );
                }
                let profile_name = profile.unwrap_or_else(get_active_profile);
                unset_profile_keys(&profile_name, api_key, workspace_id, api_url)?;
                println!("Unset keys from profile '{}'.", profile_name);
            }
        },
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
        SkillsCommands::List { all } => {
            println!("Listing skills...");
            let scope = if all {
                distri_types::stores::SkillScope::All
            } else {
                distri_types::stores::SkillScope::Workspace
            };
            let response = client
                .list_skills(&distri_types::stores::SkillFilter {
                    scope,
                    ..Default::default()
                })
                .await?;
            if response.skills.is_empty() {
                println!("No skills found.");
            } else {
                for skill in response.skills {
                    println!(
                        "{} - {}",
                        skill.name,
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
                let result = client.upsert_skill(&request).await?;
                println!(
                    "{}  Pushed skill '{}'{}",
                    COLOR_BRIGHT_GREEN, result.name, COLOR_RESET
                );
            }
        }
    }
    Ok(())
}

/// Parse a skill into a CreateSkillRequest.
///
/// Two layouts are supported:
///
/// 1. Folder layout (agentskills.io spec — preferred):
///    ```text
///    skills/my-skill/
///    ├── SKILL.md          # YAML frontmatter + body
///    ├── scripts/          # any executable files
///    │   ├── extract.py
///    │   └── merge.sh
///    ├── references/       # (warning: not yet uploaded)
///    └── assets/           # (warning: not yet uploaded)
///    ```
/// 2. Single-file legacy layout: a `.md` file with YAML frontmatter, no
///    bundled scripts.
///
/// Frontmatter follows the agentskills.io spec
/// (https://agentskills.io/specification): required `name` and `description`;
/// optional `license`, `compatibility`, `metadata`, `allowed-tools`.
pub async fn parse_skill_file(path: &Path) -> Result<CreateSkillRequest> {
    if path.is_dir() {
        return parse_skill_folder(path).await;
    }
    let raw = fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let (fm, body) = parse_skill_markdown(&raw, path)?;
    Ok(create_request(fm, body, vec![], None))
}

async fn parse_skill_folder(dir: &Path) -> Result<CreateSkillRequest> {
    let skill_md = dir.join("SKILL.md");
    if !skill_md.is_file() {
        anyhow::bail!(
            "{}: agentskills.io layout requires SKILL.md at the skill root",
            dir.display()
        );
    }
    let raw = fs::read_to_string(&skill_md)
        .await
        .with_context(|| format!("reading {}", skill_md.display()))?;
    let (fm, body) = parse_skill_markdown(&raw, &skill_md)?;

    // Per the spec, `name` must equal the parent directory name.
    let dir_name = dir.file_name().and_then(|s| s.to_str()).unwrap_or_default();
    if fm.name != dir_name {
        anyhow::bail!(
            "{}: SKILL.md `name: {}` must equal parent directory name `{}`",
            skill_md.display(),
            fm.name,
            dir_name
        );
    }

    let scripts = collect_skill_scripts(&dir.join("scripts")).await?;
    for opt in ["references", "assets"] {
        if dir.join(opt).is_dir() {
            eprintln!(
                "  warning: {}/{} skipped (not yet supported by the backend)",
                dir.display(),
                opt
            );
        }
    }
    Ok(create_request(fm, body, scripts, None))
}

fn parse_skill_markdown(
    raw: &str,
    src: &Path,
) -> Result<(distri_types::stores::SkillFrontmatter, String)> {
    let trimmed = raw.trim_start();
    let rest = trimmed
        .strip_prefix("---")
        .ok_or_else(|| anyhow::anyhow!("{}: missing leading `---` frontmatter", src.display()))?;
    let end = rest
        .find("\n---")
        .ok_or_else(|| anyhow::anyhow!("{}: missing closing `---`", src.display()))?;
    let fm_str = &rest[..end];
    let body = rest[end + 4..].trim_start_matches('\n').to_string();
    let fm: distri_types::stores::SkillFrontmatter = serde_yaml::from_str(fm_str)
        .with_context(|| format!("parsing YAML frontmatter in {}", src.display()))?;
    Ok((fm, body))
}

async fn collect_skill_scripts(scripts_dir: &Path) -> Result<Vec<distri::SkillScriptInput>> {
    if !scripts_dir.is_dir() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    let mut entries = fs::read_dir(scripts_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        let language = match p.extension().and_then(|s| s.to_str()) {
            Some("py") => "python",
            Some("js") | Some("mjs") | Some("cjs") => "javascript",
            Some("ts") | Some("tsx") => "typescript",
            Some("sh") | Some("bash") => "bash",
            Some("rb") => "ruby",
            Some("go") => "go",
            Some(other) => other,
            None => "text",
        }
        .to_string();
        let code = fs::read_to_string(&p)
            .await
            .with_context(|| format!("reading {}", p.display()))?;
        out.push(distri::SkillScriptInput {
            name,
            description: None,
            code,
            language,
        });
    }
    Ok(out)
}

fn create_request(
    fm: distri_types::stores::SkillFrontmatter,
    body: String,
    scripts: Vec<distri::SkillScriptInput>,
    source: Option<distri::SkillSource>,
) -> CreateSkillRequest {
    let tags = fm.tags();
    CreateSkillRequest {
        name: fm.name,
        description: fm.description,
        content: body,
        tags,
        scripts,
        source,
    }
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

    let is_json = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    let definition = if is_json {
        client.register_agent_json(&content).await?
    } else {
        client.register_agent_markdown(&content).await?
    };

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
