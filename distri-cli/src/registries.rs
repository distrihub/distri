// External skill registries — Claude Code-style.
//
// Users add registries to `~/.distri/registries.json`; each registry has a
// type (`skillsmp` / `github` / `git` / `local` / `http`) and a source URL.
// `distri search <q>` queries every registered registry; `distri install
// <name>@<registry>` fetches the SKILL.md (+ optional scripts/) and pushes
// it to the user's workspace as a private skill, recording provenance.
//
// On first use we seed the file with two well-known registries so the
// command works out of the box even if the user has never run
// `distri registry add`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// One configured registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: RegistryKind,
    pub url: String,
    /// API key (currently only meaningful for `skillsmp`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryKind {
    /// SkillsMP REST API at https://skillsmp.com/api/v1/skills/search?q=...
    Skillsmp,
    /// GitHub repo containing one or more SKILL.md files.
    Github,
    /// Git URL (non-GitHub).
    Git,
    /// Local filesystem path (development).
    Local,
    /// Custom HTTP endpoint that returns a `RegistryManifest` JSON.
    Http,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistriesConfig {
    pub registries: Vec<Registry>,
}

/// One skill in a search result, normalized across all registry types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSkill {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// "{registry}" — set by `RegistriesConfig::search` so the user can
    /// type `<name>@<registry>` to install.
    pub registry: String,
    /// Direct URL to the raw SKILL.md (used by `distri install`).
    pub source_url: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub stars: Option<u64>,
}

impl RegistriesConfig {
    pub fn path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".distri/registries.json")
    }

    pub fn load_or_default() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            let cfg = Self::seed();
            cfg.save()?;
            return Ok(cfg);
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let raw = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, raw)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Default registries shipped with the CLI.
    fn seed() -> Self {
        Self {
            registries: vec![
                Registry {
                    name: "anthropic".into(),
                    kind: RegistryKind::Github,
                    url: "https://github.com/anthropics/skills".into(),
                    api_key: None,
                },
                Registry {
                    name: "agentskills".into(),
                    kind: RegistryKind::Github,
                    url: "https://github.com/agentskills/agentskills".into(),
                    api_key: None,
                },
            ],
        }
    }

    pub fn add(&mut self, registry: Registry) -> Result<()> {
        if self.registries.iter().any(|r| r.name == registry.name) {
            anyhow::bail!("a registry named '{}' already exists", registry.name);
        }
        self.registries.push(registry);
        self.save()
    }

    pub fn remove(&mut self, name: &str) -> Result<bool> {
        let before = self.registries.len();
        self.registries.retain(|r| r.name != name);
        let removed = self.registries.len() < before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    pub fn get(&self, name: &str) -> Option<&Registry> {
        self.registries.iter().find(|r| r.name == name)
    }

    /// Search every configured registry in parallel and merge results.
    pub async fn search(&self, query: &str) -> Vec<DiscoveredSkill> {
        let mut results = Vec::new();
        for r in &self.registries {
            match search_registry(r, query).await {
                Ok(mut found) => results.append(&mut found),
                Err(err) => {
                    eprintln!("  warning: {} search failed: {}", r.name, err);
                }
            }
        }
        results
    }
}

/// Fetch the raw SKILL.md content from a discovered skill's `source_url`.
pub async fn fetch_skill_markdown(url: &str) -> Result<String> {
    let resp = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {}", url))?;
    if !resp.status().is_success() {
        anyhow::bail!("fetch failed: {} → HTTP {}", url, resp.status());
    }
    Ok(resp.text().await?)
}

async fn search_registry(reg: &Registry, query: &str) -> Result<Vec<DiscoveredSkill>> {
    match reg.kind {
        RegistryKind::Skillsmp => search_skillsmp(reg, query).await,
        RegistryKind::Github => search_github(reg, query).await,
        RegistryKind::Http => search_http(reg, query).await,
        RegistryKind::Local | RegistryKind::Git => {
            // Not yet implemented; return empty so the rest of search keeps
            // working. `distri install` for these kinds is a follow-up.
            Ok(vec![])
        }
    }
}

async fn search_skillsmp(reg: &Registry, query: &str) -> Result<Vec<DiscoveredSkill>> {
    // https://skillsmp.com/api/v1/skills/search?q=<q>
    let url = format!(
        "{}/api/v1/skills/search?q={}",
        reg.url.trim_end_matches('/'),
        urlencoding::encode(query)
    );
    let mut req = reqwest::Client::new().get(&url);
    if let Some(key) = &reg.api_key {
        req = req.bearer_auth(key);
    }
    let resp = req.send().await.with_context(|| format!("GET {}", url))?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }
    #[derive(Deserialize)]
    struct Item {
        name: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        author: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        skill_url: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        stars: Option<u64>,
    }
    #[derive(Deserialize)]
    struct Response {
        #[serde(default)]
        skills: Vec<Item>,
        #[serde(default)]
        results: Vec<Item>,
    }
    let body: Response = resp.json().await?;
    let items = if !body.skills.is_empty() {
        body.skills
    } else {
        body.results
    };
    Ok(items
        .into_iter()
        .filter_map(|i| {
            let source_url = i.skill_url.or(i.url)?;
            Some(DiscoveredSkill {
                name: i.name,
                description: i.description,
                registry: reg.name.clone(),
                source_url,
                author: i.author,
                tags: i.tags,
                stars: i.stars,
            })
        })
        .collect())
}

async fn search_github(reg: &Registry, query: &str) -> Result<Vec<DiscoveredSkill>> {
    // Best-effort: parse `https://github.com/owner/repo` and use GitHub's
    // search API to find SKILL.md files matching the query within the repo.
    let path = reg
        .url
        .trim_start_matches("https://github.com/")
        .trim_end_matches('/');
    let mut parts = path.splitn(2, '/');
    let owner = parts.next().unwrap_or_default();
    let repo = parts.next().unwrap_or_default();
    if owner.is_empty() || repo.is_empty() {
        anyhow::bail!("not a parseable github URL: {}", reg.url);
    }

    let api = format!(
        "https://api.github.com/search/code?q={}+filename:SKILL.md+repo:{}/{}",
        urlencoding::encode(query),
        owner,
        repo,
    );
    let resp = reqwest::Client::new()
        .get(&api)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "distri-cli")
        .send()
        .await
        .with_context(|| format!("GET {}", api))?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }
    #[derive(Deserialize)]
    struct Item {
        path: String,
        html_url: String,
    }
    #[derive(Deserialize)]
    struct Response {
        #[serde(default)]
        items: Vec<Item>,
    }
    let body: Response = resp.json().await?;
    Ok(body
        .items
        .into_iter()
        .map(|i| {
            // Convert blob URL → raw URL.
            let source_url = i
                .html_url
                .replace("github.com", "raw.githubusercontent.com")
                .replace("/blob/", "/");
            // Skill name is the parent folder of SKILL.md.
            let name = i
                .path
                .rsplit('/')
                .nth(1)
                .unwrap_or(&i.path)
                .to_string();
            DiscoveredSkill {
                name,
                description: None,
                registry: reg.name.clone(),
                source_url,
                author: Some(format!("{}/{}", owner, repo)),
                tags: vec![],
                stars: None,
            }
        })
        .collect())
}

async fn search_http(reg: &Registry, query: &str) -> Result<Vec<DiscoveredSkill>> {
    let url = format!(
        "{}?q={}",
        reg.url.trim_end_matches('/'),
        urlencoding::encode(query)
    );
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {}", url))?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }
    #[derive(Deserialize)]
    struct Manifest {
        skills: Vec<DiscoveredSkill>,
    }
    let m: Manifest = resp.json().await?;
    Ok(m.skills
        .into_iter()
        .map(|mut s| {
            s.registry = reg.name.clone();
            s
        })
        .collect())
}
