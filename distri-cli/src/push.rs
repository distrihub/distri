// distri push / checkout / search / install — unified author-side workflow.
//
// `distri push [path]`     walks the project (or one subtree) and uploads
//                          agents, skills, and templates in one pass.
// `distri checkout`        materializes the workspace's content back to
//                          the same convention layout (agents/, skills/,
//                          templates/) so a user can edit + re-push.
// `distri search <q>`      searches every registered external registry
//                          (skillsmp.com, GitHub, …) for matching skills.
// `distri install <n>@<r>` fetches a skill from a registry and pushes it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use distri::Distri;
use tokio::fs;

use crate::commands::parse_skill_file;
use crate::registries::{
    fetch_skill_markdown, DiscoveredSkill, RegistriesConfig, Registry, RegistryKind,
};
use crate::{CheckoutScope, RegistryCommands};

const COLOR_BRIGHT_GREEN: &str = "\x1b[92m";
const COLOR_YELLOW: &str = "\x1b[93m";
const COLOR_RESET: &str = "\x1b[0m";

// ─── push ────────────────────────────────────────────────────────────

pub async fn handle_push(client: &Distri, path: Option<PathBuf>, dry_run: bool) -> Result<()> {
    let root = path.unwrap_or_else(|| PathBuf::from("."));
    if !root.exists() {
        anyhow::bail!("path does not exist: {}", root.display());
    }

    // If the user passed `agents/`, `skills/`, or `templates/` (or any
    // subpath inside them), scope to that resource.
    if let Some(name) = root.file_name().and_then(|s| s.to_str()) {
        match name {
            "agents" if root.is_dir() => return push_agents_dir(client, &root, dry_run).await,
            "skills" if root.is_dir() => return push_skills_dir(client, &root, dry_run).await,
            "templates" if root.is_dir() => {
                return push_templates_dir(client, &root, dry_run).await
            }
            _ => {}
        }
    }

    // Single-file or single-skill-folder push.
    if root.is_file() {
        return push_single_file(client, &root, dry_run).await;
    }
    if is_skill_folder(&root) {
        return push_skill_folder(client, &root, dry_run).await;
    }

    // Otherwise treat as a project root.
    push_project(client, &root, dry_run).await
}

async fn push_project(client: &Distri, root: &Path, dry_run: bool) -> Result<()> {
    let agents = root.join("agents");
    let skills = root.join("skills");
    let templates = root.join("templates");

    let mut found = false;
    if agents.is_dir() {
        push_agents_dir(client, &agents, dry_run).await?;
        found = true;
    }
    if skills.is_dir() {
        push_skills_dir(client, &skills, dry_run).await?;
        found = true;
    }
    if templates.is_dir() {
        push_templates_dir(client, &templates, dry_run).await?;
        found = true;
    }
    if !found {
        eprintln!(
            "  warning: no agents/, skills/, or templates/ directories found under {}",
            root.display()
        );
    }
    Ok(())
}

async fn push_agents_dir(client: &Distri, dir: &Path, dry_run: bool) -> Result<()> {
    let mut entries = fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        match p.extension().and_then(|s| s.to_str()) {
            Some("md") | Some("json") => push_agent_file(client, &p, dry_run).await?,
            _ => {}
        }
    }
    Ok(())
}

async fn push_agent_file(client: &Distri, path: &Path, dry_run: bool) -> Result<()> {
    if dry_run {
        println!("  [dry-run] agent {}", path.display());
        return Ok(());
    }
    let raw = fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let resp = if path.extension().and_then(|s| s.to_str()) == Some("json") {
        client.register_agent_json(&raw).await?
    } else {
        client.register_agent_markdown(&raw).await?
    };
    println!(
        "{}  Pushed agent '{}'{}",
        COLOR_BRIGHT_GREEN, resp.name, COLOR_RESET
    );
    Ok(())
}

async fn push_skills_dir(client: &Distri, dir: &Path, dry_run: bool) -> Result<()> {
    let mut entries = fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let p = entry.path();
        if p.is_dir() && is_skill_folder(&p) {
            push_skill_folder(client, &p, dry_run).await?;
        } else if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("md") {
            // Legacy single-file skill (pre-agentskills.io layout).
            push_skill_single_file(client, &p, dry_run).await?;
        }
    }
    Ok(())
}

fn is_skill_folder(p: &Path) -> bool {
    p.is_dir() && p.join("SKILL.md").is_file()
}

async fn push_skill_folder(client: &Distri, folder: &Path, dry_run: bool) -> Result<()> {
    if dry_run {
        println!("  [dry-run] skill {}", folder.display());
        return Ok(());
    }
    let req = parse_skill_file(folder).await?;
    let result = client.upsert_skill(&req).await?;
    println!(
        "{}  Pushed skill '{}' ({} script{}){}",
        COLOR_BRIGHT_GREEN,
        result.name,
        req.scripts.len(),
        if req.scripts.len() == 1 { "" } else { "s" },
        COLOR_RESET
    );
    Ok(())
}

async fn push_skill_single_file(client: &Distri, path: &Path, dry_run: bool) -> Result<()> {
    if dry_run {
        println!("  [dry-run] skill {}", path.display());
        return Ok(());
    }
    let req = parse_skill_file(path).await?;
    let result = client.upsert_skill(&req).await?;
    println!(
        "{}  Pushed skill '{}'{}",
        COLOR_BRIGHT_GREEN, result.name, COLOR_RESET
    );
    Ok(())
}

async fn push_templates_dir(client: &Distri, dir: &Path, dry_run: bool) -> Result<()> {
    let mut entries = fs::read_dir(dir).await?;
    let mut templates = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let ext = p.extension().and_then(|s| s.to_str());
        if !matches!(ext, Some("hbs") | Some("handlebars")) {
            continue;
        }
        let name = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let body = fs::read_to_string(&p)
            .await
            .with_context(|| format!("reading {}", p.display()))?;
        if dry_run {
            println!("  [dry-run] template {}", p.display());
            continue;
        }
        templates.push(distri::NewPromptTemplateRequest {
            name,
            description: None,
            template: body,
            version: None,
        });
    }
    if dry_run || templates.is_empty() {
        return Ok(());
    }
    let resp = client.sync_prompt_templates(&templates).await?;
    println!(
        "{}  Pushed {} template(s){}",
        COLOR_BRIGHT_GREEN,
        resp.created + resp.updated,
        COLOR_RESET
    );
    Ok(())
}

async fn push_single_file(client: &Distri, path: &Path, dry_run: bool) -> Result<()> {
    let ext = path.extension().and_then(|s| s.to_str());
    match ext {
        Some("hbs") | Some("handlebars") => {
            // Templates synced one-shot; reuse the dir helper on its parent.
            let parent = path.parent().unwrap_or(Path::new("."));
            push_templates_dir(client, parent, dry_run).await
        }
        Some("json") => push_agent_file(client, path, dry_run).await,
        Some("md") => {
            // Disambiguate: agent vs skill. Skills must have folder layout
            // for the new flow; standalone .md is treated as an agent.
            push_agent_file(client, path, dry_run).await
        }
        _ => anyhow::bail!(
            "{} — unrecognized file type. Expected .md / .json (agent), \
             .hbs (template), or a skill folder containing SKILL.md.",
            path.display()
        ),
    }
}

// ─── checkout ────────────────────────────────────────────────────────

pub async fn handle_checkout(
    client: &Distri,
    out: Option<PathBuf>,
    scope: CheckoutScope,
    force: bool,
) -> Result<()> {
    let root = out.unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&root).await?;
    if !force && fs::read_dir(&root).await?.next_entry().await?.is_some() {
        anyhow::bail!(
            "{} is not empty. Re-run with --force to overwrite.",
            root.display()
        );
    }

    let want = |t: CheckoutScope| matches!(scope, CheckoutScope::All) || scope == t;

    if want(CheckoutScope::Agents) {
        checkout_agents(client, &root.join("agents")).await?;
    }
    if want(CheckoutScope::Skills) {
        checkout_skills(client, &root.join("skills")).await?;
    }
    if want(CheckoutScope::Templates) {
        checkout_templates(client, &root.join("templates")).await?;
    }
    Ok(())
}

async fn checkout_agents(client: &Distri, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).await?;
    let agents = client.list_agents().await?;
    for agent in agents {
        match client.fetch_agent(&agent.name).await {
            Ok(Some(cfg)) => {
                if let Some(md) = cfg.markdown.as_deref() {
                    let path = dir.join(format!("{}.md", agent.name));
                    fs::write(&path, md).await?;
                    println!(
                        "{}  Wrote agent {}{}",
                        COLOR_BRIGHT_GREEN,
                        path.display(),
                        COLOR_RESET
                    );
                } else {
                    let path = dir.join(format!("{}.json", agent.name));
                    let json = serde_json::to_string_pretty(&cfg.agent)?;
                    fs::write(&path, json).await?;
                    println!(
                        "{}  Wrote agent {}{}",
                        COLOR_BRIGHT_GREEN,
                        path.display(),
                        COLOR_RESET
                    );
                }
            }
            Ok(None) => eprintln!("  warning: agent '{}' disappeared mid-checkout", agent.name),
            Err(err) => eprintln!("  warning: failed to fetch '{}': {}", agent.name, err),
        }
    }
    Ok(())
}

async fn checkout_skills(client: &Distri, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).await?;
    let filter = distri_types::stores::SkillFilter {
        scope: distri_types::stores::SkillScope::Workspace,
        ..Default::default()
    };
    let resp = client.list_skills(&filter).await?;
    for s in resp.skills {
        let skill_dir = dir.join(&s.name);
        fs::create_dir_all(&skill_dir).await?;
        // Best-effort: re-fetch full record for content.
        if let Some(full) = client.get_skill(&s.id).await? {
            let mut md = String::from("---\n");
            md.push_str(&format!("name: {}\n", full.name));
            if let Some(d) = &full.description {
                md.push_str(&format!("description: {}\n", d));
            }
            md.push_str("---\n\n");
            md.push_str(&full.content);
            let skill_md = skill_dir.join("SKILL.md");
            fs::write(&skill_md, md).await?;
            println!(
                "{}  Wrote skill {}{}",
                COLOR_BRIGHT_GREEN,
                skill_md.display(),
                COLOR_RESET
            );
        }
    }
    Ok(())
}

async fn checkout_templates(client: &Distri, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).await?;
    let templates = client.list_prompt_templates().await?;
    for t in templates {
        let path = dir.join(format!("{}.hbs", t.name));
        fs::write(&path, &t.template).await?;
        println!(
            "{}  Wrote template {}{}",
            COLOR_BRIGHT_GREEN,
            path.display(),
            COLOR_RESET
        );
    }
    Ok(())
}

// ─── search ──────────────────────────────────────────────────────────

pub async fn handle_search(query: String, registry: Option<String>) -> Result<()> {
    let cfg = RegistriesConfig::load_or_default()?;
    let to_search: Vec<&Registry> = match registry {
        Some(name) => cfg.registries.iter().filter(|r| r.name == name).collect(),
        None => cfg.registries.iter().collect(),
    };
    if to_search.is_empty() {
        anyhow::bail!("no matching registries configured");
    }

    let mut results: Vec<DiscoveredSkill> = Vec::new();
    for r in &to_search {
        let one = single_registry_search(r, &query).await;
        results.extend(one);
    }
    if results.is_empty() {
        println!("No matches.");
        return Ok(());
    }
    for r in results {
        println!(
            "{}@{} — {}",
            r.name,
            r.registry,
            r.description.as_deref().unwrap_or("(no description)")
        );
    }
    Ok(())
}

async fn single_registry_search(r: &Registry, query: &str) -> Vec<DiscoveredSkill> {
    let cfg = RegistriesConfig {
        registries: vec![r.clone()],
    };
    cfg.search(query).await
}

// ─── install ─────────────────────────────────────────────────────────

pub async fn handle_install(client: &Distri, reference: &str) -> Result<()> {
    let (name, registry_name) = reference.rsplit_once('@').ok_or_else(|| {
        anyhow::anyhow!(
            "expected `<name>@<registry>` (got `{}`); run `distri registry list` to see registries",
            reference
        )
    })?;
    let cfg = RegistriesConfig::load_or_default()?;
    let registry = cfg
        .get(registry_name)
        .ok_or_else(|| anyhow::anyhow!("registry '{}' not configured", registry_name))?;

    println!(
        "{}Installing {} from {}…{}",
        COLOR_YELLOW, name, registry_name, COLOR_RESET
    );

    // Search the registry for an exact-name match, then fetch + push.
    let matches: Vec<DiscoveredSkill> = single_registry_search(registry, name).await;
    let chosen = matches
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| anyhow::anyhow!("'{}' not found in registry '{}'", name, registry_name))?;

    let raw = fetch_skill_markdown(&chosen.source_url).await?;
    let (fm, body) = parse_yaml_frontmatter(&raw, &chosen.source_url)?;
    let req = distri::CreateSkillRequest {
        name: fm.name.clone(),
        description: fm.description.clone(),
        content: body,
        tags: fm.tags(),
        scripts: vec![],
        source: Some(distri::SkillSource {
            registry: registry_name.to_string(),
            url: chosen.source_url,
            git_ref: None,
        }),
    };
    let result = client.upsert_skill(&req).await?;
    println!(
        "{}  Installed skill '{}'{}",
        COLOR_BRIGHT_GREEN, result.name, COLOR_RESET
    );
    Ok(())
}

fn parse_yaml_frontmatter(
    raw: &str,
    src: &str,
) -> Result<(distri_types::stores::SkillFrontmatter, String)> {
    let trimmed = raw.trim_start();
    let rest = trimmed
        .strip_prefix("---")
        .ok_or_else(|| anyhow::anyhow!("{}: missing leading `---` frontmatter", src))?;
    let end = rest
        .find("\n---")
        .ok_or_else(|| anyhow::anyhow!("{}: missing closing `---`", src))?;
    let fm_str = &rest[..end];
    let body = rest[end + 4..].trim_start_matches('\n').to_string();
    let fm: distri_types::stores::SkillFrontmatter = serde_yaml::from_str(fm_str)
        .with_context(|| format!("parsing YAML frontmatter from {}", src))?;
    Ok((fm, body))
}

// ─── registry add/remove/list ────────────────────────────────────────

pub fn handle_registry(cmd: RegistryCommands) -> Result<()> {
    match cmd {
        RegistryCommands::List => {
            let cfg = RegistriesConfig::load_or_default()?;
            if cfg.registries.is_empty() {
                println!("No registries configured.");
                return Ok(());
            }
            for r in &cfg.registries {
                println!(
                    "{:<20} {:<10} {}",
                    r.name,
                    format!("{:?}", r.kind).to_lowercase(),
                    r.url
                );
            }
        }
        RegistryCommands::Add {
            name,
            kind,
            url,
            api_key,
        } => {
            let kind = parse_registry_kind(&kind)?;
            let mut cfg = RegistriesConfig::load_or_default()?;
            cfg.add(Registry {
                name: name.clone(),
                kind,
                url,
                api_key,
            })?;
            println!(
                "{}  Added registry '{}'{}",
                COLOR_BRIGHT_GREEN, name, COLOR_RESET
            );
        }
        RegistryCommands::Remove { name } => {
            let mut cfg = RegistriesConfig::load_or_default()?;
            if cfg.remove(&name)? {
                println!(
                    "{}  Removed registry '{}'{}",
                    COLOR_BRIGHT_GREEN, name, COLOR_RESET
                );
            } else {
                println!("No registry named '{}'", name);
            }
        }
    }
    Ok(())
}

fn parse_registry_kind(s: &str) -> Result<RegistryKind> {
    match s.to_lowercase().as_str() {
        "skillsmp" => Ok(RegistryKind::Skillsmp),
        "github" => Ok(RegistryKind::Github),
        "git" => Ok(RegistryKind::Git),
        "local" => Ok(RegistryKind::Local),
        "http" => Ok(RegistryKind::Http),
        _ => anyhow::bail!(
            "unknown registry kind '{}' (use skillsmp/github/git/local/http)",
            s
        ),
    }
}
