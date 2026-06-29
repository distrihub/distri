# CLI Profiles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single `~/.distri/config` credential store with a named-profile system (AWS-style INI `~/.distri/credentials`), with full CLI management under `distri profile`.

**Architecture:** A new `credentials.rs` module in `distri-cli` owns all profile file I/O — INI read/write, active-profile tracking, and legacy migration. `DistriConfig` loading in `main.rs` is replaced with `load_config_with_profile()` from `credentials.rs` that merges the active profile with env var overrides. All profile commands live under `Commands::Profile` in `main.rs` / `handle_profile_command` in `commands.rs`.

**Tech Stack:** Rust, hand-written INI parser (no new dep), existing `toml` crate for `~/.distri/config`, `clap` for CLI.

---

## File Map

| Action | File | Responsibility |
|--------|------|----------------|
| Create | `distri-cli/src/credentials.rs` | INI parse/write, `ProfileValues`, `list_profiles`, `load_profile`, `save_profile`, `delete_profile`, `get_active_profile`, `set_active_profile`, `load_config_with_profile`, `migrate_legacy_config` |
| Modify | `distri-cli/src/commands.rs` | Remove `handle_config_command`, add `handle_profile_command` |
| Modify | `distri-cli/src/main.rs` | Replace `Commands::Config` with `Commands::Profile`, add `--profile` to `Login`, wire `load_config_with_profile()` |
| Modify | `distri-cli/src/login.rs` | Accept `profile_name: &str`, save to `credentials.rs` instead of TOML config |
| Modify | `distri-cli/src/config.rs` | Remove `set_client_config_value` and `load_client_config_value` |

---

### Task 1: Create `credentials.rs` — INI parser and ProfileValues

**Files:**
- Create: `distri/distri-cli/src/credentials.rs`
- Test: inside the same file under `#[cfg(test)]`

- [ ] **Step 1: Write the failing tests**

Add to `distri/distri-cli/src/credentials.rs`:

```rust
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;

const CREDENTIALS_FILE_NAME: &str = "credentials";
const CONFIG_FILE_NAME: &str = "config";
const CONFIG_DIR_NAME: &str = ".distri";
const ENV_PROFILE: &str = "DISTRI_PROFILE";
const DEFAULT_PROFILE: &str = "default";
const DEFAULT_API_URL: &str = "https://api.distri.dev/v1";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfileValues {
    pub api_key: Option<String>,
    pub workspace_id: Option<String>,
    pub api_url: Option<String>,
}

fn distri_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(CONFIG_DIR_NAME))
}

pub fn credentials_path() -> Option<PathBuf> {
    Some(distri_dir()?.join(CREDENTIALS_FILE_NAME))
}

pub fn config_path() -> Option<PathBuf> {
    Some(distri_dir()?.join(CONFIG_FILE_NAME))
}

// ---------------------------------------------------------------------------
// INI parser — handles [section] headers and key = value lines.
// Lines starting with # or ; are comments. Blank lines are ignored.
// ---------------------------------------------------------------------------

type IniData = BTreeMap<String, BTreeMap<String, String>>;

fn parse_ini(content: &str) -> IniData {
    let mut data: IniData = BTreeMap::new();
    let mut current_section = String::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].trim().to_string();
            data.entry(current_section.clone()).or_default();
        } else if let Some((k, v)) = line.split_once('=') {
            let key = k.trim().to_string();
            let value = v.trim().to_string();
            if !current_section.is_empty() && !key.is_empty() {
                data.entry(current_section.clone())
                    .or_default()
                    .insert(key, value);
            }
        }
    }
    data
}

fn serialize_ini(data: &IniData) -> String {
    let mut out = String::new();
    for (section, kv) in data {
        out.push_str(&format!("[{}]\n", section));
        for (k, v) in kv {
            out.push_str(&format!("{} = {}\n", k, v));
        }
        out.push('\n');
    }
    out
}

fn read_ini(path: &PathBuf) -> IniData {
    std::fs::read_to_string(path)
        .map(|s| parse_ini(&s))
        .unwrap_or_default()
}

fn write_ini(path: &PathBuf, data: &IniData) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serialize_ini(data))?;
    Ok(())
}

fn section_to_profile(section: &BTreeMap<String, String>) -> ProfileValues {
    ProfileValues {
        api_key: section.get("api_key").cloned(),
        workspace_id: section.get("workspace_id").cloned(),
        api_url: section.get("api_url").cloned(),
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn list_profiles() -> Result<Vec<(String, ProfileValues)>> {
    let path = credentials_path()
        .context("Unable to resolve home directory")?;
    let data = read_ini(&path);
    Ok(data
        .into_iter()
        .map(|(name, section)| (name, section_to_profile(&section)))
        .collect())
}

pub fn load_profile(name: &str) -> Result<Option<ProfileValues>> {
    let path = credentials_path()
        .context("Unable to resolve home directory")?;
    let data = read_ini(&path);
    Ok(data.get(name).map(|s| section_to_profile(s)))
}

/// Merge-save: only updates keys that are Some in `values`, leaves others untouched.
pub fn save_profile(name: &str, values: &ProfileValues) -> Result<()> {
    let path = credentials_path()
        .context("Unable to resolve home directory")?;
    let mut data = read_ini(&path);
    let section = data.entry(name.to_string()).or_default();
    if let Some(ref v) = values.api_key {
        section.insert("api_key".to_string(), v.clone());
    }
    if let Some(ref v) = values.workspace_id {
        section.insert("workspace_id".to_string(), v.clone());
    }
    if let Some(ref v) = values.api_url {
        section.insert("api_url".to_string(), v.clone());
    }
    write_ini(&path, &data)
}

/// Remove specific keys from a profile (keys with `true` are removed).
pub fn unset_profile_keys(
    name: &str,
    api_key: bool,
    workspace_id: bool,
    api_url: bool,
) -> Result<()> {
    let path = credentials_path()
        .context("Unable to resolve home directory")?;
    let mut data = read_ini(&path);
    if let Some(section) = data.get_mut(name) {
        if api_key { section.remove("api_key"); }
        if workspace_id { section.remove("workspace_id"); }
        if api_url { section.remove("api_url"); }
    }
    write_ini(&path, &data)
}

pub fn delete_profile(name: &str) -> Result<()> {
    let path = credentials_path()
        .context("Unable to resolve home directory")?;
    let mut data = read_ini(&path);
    data.remove(name);
    write_ini(&path, &data)
}

pub fn get_active_profile() -> String {
    // Env var takes precedence over config file
    if let Ok(p) = std::env::var(ENV_PROFILE) {
        let p = p.trim().to_string();
        if !p.is_empty() {
            return p;
        }
    }
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| {
            s.lines()
                .find_map(|line| {
                    let line = line.trim();
                    line.strip_prefix("active_profile")
                        .and_then(|rest| rest.trim().strip_prefix('='))
                        .map(|v| v.trim().trim_matches('"').to_string())
                })
        })
        .unwrap_or_else(|| DEFAULT_PROFILE.to_string())
}

pub fn set_active_profile(name: &str) -> Result<()> {
    let path = config_path()
        .context("Unable to resolve home directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Read existing config, update or insert active_profile line
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
    let new_line = format!("active_profile = \"{}\"", name);
    let pos = lines.iter().position(|l| l.trim().starts_with("active_profile"));
    match pos {
        Some(i) => lines[i] = new_line,
        None => lines.push(new_line),
    }
    std::fs::write(&path, lines.join("\n") + "\n")?;
    Ok(())
}

/// Migrate legacy `~/.distri/config` (api_key, workspace_id, base_url keys) to
/// the [default] profile in `~/.distri/credentials`, then remove those keys from config.
pub fn migrate_legacy_config() -> Result<()> {
    let config_path = match config_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    let creds_path = match credentials_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    // Only migrate if credentials file doesn't exist yet
    if creds_path.exists() {
        return Ok(());
    }
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    // Parse TOML-style config for legacy keys
    let mut api_key: Option<String> = None;
    let mut workspace_id: Option<String> = None;
    let mut base_url: Option<String> = None;
    let mut remaining_lines: Vec<String> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("api_key") {
            if let Some(v) = rest.trim().strip_prefix('=') {
                api_key = Some(v.trim().trim_matches('"').to_string());
                continue;
            }
        }
        if let Some(rest) = trimmed.strip_prefix("workspace_id") {
            if let Some(v) = rest.trim().strip_prefix('=') {
                workspace_id = Some(v.trim().trim_matches('"').to_string());
                continue;
            }
        }
        if let Some(rest) = trimmed.strip_prefix("base_url") {
            if let Some(v) = rest.trim().strip_prefix('=') {
                base_url = Some(v.trim().trim_matches('"').to_string());
                continue;
            }
        }
        remaining_lines.push(line.to_string());
    }
    if api_key.is_some() || workspace_id.is_some() || base_url.is_some() {
        let values = ProfileValues {
            api_key,
            workspace_id,
            api_url: base_url,
        };
        save_profile(DEFAULT_PROFILE, &values)?;
        // Rewrite config without the migrated keys
        std::fs::write(&config_path, remaining_lines.join("\n") + "\n")?;
    }
    Ok(())
}

/// Build a DistriConfig by merging: env vars > active profile > defaults.
/// Call this instead of DistriConfig::from_env() in the CLI.
pub fn load_config_with_profile() -> distri_types::DistriConfig {
    use distri_types::DistriConfig;
    let profile_name = get_active_profile();
    let profile = load_profile(&profile_name).unwrap_or_default().unwrap_or_default();

    let env_api_key = std::env::var("DISTRI_API_KEY").ok().filter(|s| !s.is_empty());
    let env_workspace_id = std::env::var("DISTRI_WORKSPACE_ID").ok().filter(|s| !s.is_empty());
    let env_base_url = std::env::var("DISTRI_BASE_URL").ok().filter(|s| !s.is_empty());

    let base_url = env_base_url
        .or(profile.api_url)
        .unwrap_or_else(|| DEFAULT_API_URL.to_string());
    let api_key = env_api_key.or(profile.api_key);
    let workspace_id = env_workspace_id.or(profile.workspace_id);

    DistriConfig::new(base_url)
        .with_maybe_api_key(api_key)
        .with_maybe_workspace_id(workspace_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn with_temp_dir(f: impl FnOnce(&TempDir)) {
        let dir = TempDir::new().unwrap();
        f(&dir);
    }

    fn temp_creds_path(dir: &TempDir) -> PathBuf {
        dir.path().join("credentials")
    }

    fn temp_config_path(dir: &TempDir) -> PathBuf {
        dir.path().join("config")
    }

    // Helper: parse/serialize round-trip
    #[test]
    fn test_ini_round_trip() {
        let input = "[default]\napi_key = dak_abc\nworkspace_id = 1234\n\n[local]\napi_url = http://localhost:8080/v1\n";
        let data = parse_ini(input);
        assert_eq!(data["default"]["api_key"], "dak_abc");
        assert_eq!(data["local"]["api_url"], "http://localhost:8080/v1");
        let out = serialize_ini(&data);
        let reparsed = parse_ini(&out);
        assert_eq!(data, reparsed);
    }

    #[test]
    fn test_save_load_profile() {
        with_temp_dir(|dir| {
            let path = temp_creds_path(dir);
            // Manually call write_ini via save_profile using overridden path
            let mut data: IniData = BTreeMap::new();
            let values = ProfileValues {
                api_key: Some("dak_test".to_string()),
                workspace_id: Some("ws-123".to_string()),
                api_url: Some("https://api.distri.dev/v1".to_string()),
            };
            let section = data.entry("default".to_string()).or_default();
            section.insert("api_key".to_string(), "dak_test".to_string());
            section.insert("workspace_id".to_string(), "ws-123".to_string());
            section.insert("api_url".to_string(), "https://api.distri.dev/v1".to_string());
            write_ini(&path, &data).unwrap();
            let loaded = read_ini(&path);
            let profile = section_to_profile(&loaded["default"]);
            assert_eq!(profile, values);
        });
    }

    #[test]
    fn test_merge_save_preserves_existing_keys() {
        with_temp_dir(|dir| {
            let path = temp_creds_path(dir);
            // Write initial full profile
            let mut data: IniData = BTreeMap::new();
            let section = data.entry("default".to_string()).or_default();
            section.insert("api_key".to_string(), "old_key".to_string());
            section.insert("workspace_id".to_string(), "old_ws".to_string());
            section.insert("api_url".to_string(), "https://api.distri.dev/v1".to_string());
            write_ini(&path, &data).unwrap();

            // Update only api_key by reading and merging
            let mut existing = read_ini(&path);
            let s = existing.entry("default".to_string()).or_default();
            s.insert("api_key".to_string(), "new_key".to_string());
            write_ini(&path, &existing).unwrap();

            let result = read_ini(&path);
            assert_eq!(result["default"]["api_key"], "new_key");
            assert_eq!(result["default"]["workspace_id"], "old_ws"); // preserved
            assert_eq!(result["default"]["api_url"], "https://api.distri.dev/v1"); // preserved
        });
    }

    #[test]
    fn test_delete_profile() {
        with_temp_dir(|dir| {
            let path = temp_creds_path(dir);
            let mut data: IniData = BTreeMap::new();
            data.entry("default".to_string()).or_default().insert("api_key".to_string(), "k".to_string());
            data.entry("local".to_string()).or_default().insert("api_key".to_string(), "k2".to_string());
            write_ini(&path, &data).unwrap();
            let mut d = read_ini(&path);
            d.remove("local");
            write_ini(&path, &d).unwrap();
            let result = read_ini(&path);
            assert!(result.contains_key("default"));
            assert!(!result.contains_key("local"));
        });
    }

    #[test]
    fn test_migrate_legacy_config() {
        with_temp_dir(|dir| {
            let config = temp_config_path(dir);
            let creds = temp_creds_path(dir);
            std::fs::write(&config, "api_key = \"dak_old\"\nworkspace_id = \"ws-old\"\n").unwrap();
            // Run migration inline (can't use global paths in unit test, so test the logic directly)
            let content = std::fs::read_to_string(&config).unwrap();
            let mut api_key: Option<String> = None;
            let mut workspace_id: Option<String> = None;
            let mut remaining: Vec<String> = Vec::new();
            for line in content.lines() {
                let t = line.trim();
                if let Some(rest) = t.strip_prefix("api_key") {
                    if let Some(v) = rest.trim().strip_prefix('=') {
                        api_key = Some(v.trim().trim_matches('"').to_string());
                        continue;
                    }
                }
                if let Some(rest) = t.strip_prefix("workspace_id") {
                    if let Some(v) = rest.trim().strip_prefix('=') {
                        workspace_id = Some(v.trim().trim_matches('"').to_string());
                        continue;
                    }
                }
                remaining.push(line.to_string());
            }
            assert_eq!(api_key.as_deref(), Some("dak_old"));
            assert_eq!(workspace_id.as_deref(), Some("ws-old"));
            // Write to creds
            let mut data: IniData = BTreeMap::new();
            let s = data.entry("default".to_string()).or_default();
            s.insert("api_key".to_string(), api_key.unwrap());
            s.insert("workspace_id".to_string(), workspace_id.unwrap());
            write_ini(&creds, &data).unwrap();
            std::fs::write(&config, remaining.join("\n") + "\n").unwrap();
            // Verify
            let migrated = read_ini(&creds);
            assert_eq!(migrated["default"]["api_key"], "dak_old");
            let new_config = std::fs::read_to_string(&config).unwrap();
            assert!(!new_config.contains("api_key"));
        });
    }
}
```

- [ ] **Step 2: Run tests (expect compile error — `with_maybe_api_key` doesn't exist yet)**

```bash
cd distri && cargo test -p distri-cli credentials 2>&1 | head -30
```

Expected: compile error about `with_maybe_api_key` / `with_maybe_workspace_id`.

- [ ] **Step 3: Add `with_maybe_*` helpers to `DistriConfig` in `distri-types/src/client_config.rs`**

Add after the existing `with_workspace_id` method:

```rust
/// Set the API key if Some.
pub fn with_maybe_api_key(mut self, api_key: Option<String>) -> Self {
    self.api_key = api_key;
    self
}

/// Set the workspace ID if Some.
pub fn with_maybe_workspace_id(mut self, workspace_id: Option<String>) -> Self {
    self.workspace_id = workspace_id;
    self
}
```

- [ ] **Step 4: Add `credentials` module to `distri-cli/src/main.rs`**

Add to the module declarations at the top of `main.rs`:

```rust
mod credentials;
```

- [ ] **Step 5: Run tests — expect them to pass**

```bash
cd distri && cargo test -p distri-cli credentials 2>&1
```

Expected: all `credentials::tests::*` pass.

- [ ] **Step 6: Commit**

```bash
cd distri && git add distri-cli/src/credentials.rs distri-cli/src/main.rs distri-types/src/client_config.rs
git commit -m "feat: add credentials.rs with INI profile I/O and DistriConfig helpers"
```

---

### Task 2: Wire `load_config_with_profile()` into main.rs + run migration

**Files:**
- Modify: `distri/distri-cli/src/main.rs`

- [ ] **Step 1: Replace `DistriConfig::from_env()` call in `main.rs`**

In `main.rs`, find (around line 357):
```rust
let mut config = DistriConfig::from_env();
```

Replace with:
```rust
// Run one-time migration of legacy ~/.distri/config keys to ~/.distri/credentials
let _ = crate::credentials::migrate_legacy_config();
let mut config = crate::credentials::load_config_with_profile();
```

- [ ] **Step 2: Verify it compiles**

```bash
cd distri && cargo check -p distri-cli 2>&1 | grep "^error"
```

Expected: no errors.

- [ ] **Step 3: Smoke test — running distri still loads config**

```bash
cd distri && cargo run -p distri-cli -- --help 2>&1 | head -5
```

Expected: help text printed without panic.

- [ ] **Step 4: Commit**

```bash
cd distri && git add distri-cli/src/main.rs
git commit -m "feat: wire load_config_with_profile into CLI startup with legacy migration"
```

---

### Task 3: Replace `Commands::Config` with `Commands::Profile` in main.rs

**Files:**
- Modify: `distri/distri-cli/src/main.rs`

- [ ] **Step 1: Remove `ConfigCommands` and add `ProfileCommands` enums**

In `main.rs`, replace the `ConfigCommands` block:

```rust
// REMOVE this entire block:
#[derive(Subcommand, Debug, Clone)]
pub(crate) enum ConfigCommands {
    Set {
        #[clap(help = "Config key (api_key, base_url, workspace_id)")]
        key: String,
        #[clap(help = "Value to set (empty clears the key)", num_args = 1..)]
        value: Vec<String>,
    },
}
```

Add new enums:

```rust
#[derive(Subcommand, Debug, Clone)]
pub(crate) enum ProfileCommands {
    /// List all profiles
    List,
    /// Set the active profile
    Use {
        #[clap(help = "Profile name")]
        name: String,
    },
    /// Show profile values (active profile if no name given)
    Show {
        #[clap(help = "Profile name (defaults to active)")]
        name: Option<String>,
    },
    /// Delete a profile
    Delete {
        #[clap(help = "Profile name")]
        name: String,
        #[clap(long, short, help = "Skip confirmation prompt")]
        yes: bool,
    },
    /// Manage credential keys within a profile
    Config {
        #[clap(subcommand)]
        command: ProfileConfigCommands,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum ProfileConfigCommands {
    /// Set one or more credential keys on a profile
    Set {
        #[clap(long, help = "Target profile (defaults to active)")]
        profile: Option<String>,
        #[clap(long, help = "API key")]
        api_key: Option<String>,
        #[clap(long, help = "Workspace ID (UUID)")]
        workspace_id: Option<String>,
        #[clap(long, help = "API URL")]
        api_url: Option<String>,
    },
    /// Remove one or more credential keys from a profile
    Unset {
        #[clap(long, help = "Target profile (defaults to active)")]
        profile: Option<String>,
        #[clap(long, help = "Remove api_key")]
        api_key: bool,
        #[clap(long, help = "Remove workspace_id")]
        workspace_id: bool,
        #[clap(long, help = "Remove api_url")]
        api_url: bool,
    },
}
```

- [ ] **Step 2: Replace `Commands::Config` variant with `Commands::Profile`, add `--profile` to `Login`**

In the `Commands` enum, replace:
```rust
/// Manage local client configuration
Config {
    #[clap(subcommand)]
    command: ConfigCommands,
},
```
With:
```rust
/// Manage authentication profiles
Profile {
    #[clap(subcommand)]
    command: ProfileCommands,
},
```

And update `Commands::Login`:
```rust
/// Login to Distri Cloud and configure workspace
Login {
    #[clap(long, help = "Email address")]
    email: Option<String>,
    #[clap(long, help = "Skip workspace selection (use default)")]
    skip_workspace: bool,
    #[clap(long, help = "Profile name to save credentials into (default: \"default\")")]
    profile: Option<String>,
},
```

- [ ] **Step 3: Update the `match command` block in `main.rs`**

Find the `Commands::Config` match arm:
```rust
Commands::Config { command } => {
    handle_config_command(command)?;
}
```
Replace with:
```rust
Commands::Profile { command } => {
    handle_profile_command(command)?;
}
```

Find the `Commands::Login` match arm and add `profile`:
```rust
Commands::Login { email, skip_workspace, profile } => {
    login::handle_login_command(email, skip_workspace, profile).await?;
}
```

- [ ] **Step 4: Update imports in `main.rs`** — remove `handle_config_command` from the `commands::` import, add `handle_profile_command`:

```rust
use commands::{
    handle_connections_command, handle_profile_command, handle_prompts_command,
    handle_secrets_command, handle_skills_command, handle_workflow_command, push_file,
};
```

- [ ] **Step 5: Verify compile (will fail until handle_profile_command exists)**

```bash
cd distri && cargo check -p distri-cli 2>&1 | grep "^error" | head -10
```

Expected: error about `handle_profile_command` not found — that's correct, implement it next.

---

### Task 4: Implement `handle_profile_command` in `commands.rs`

**Files:**
- Modify: `distri/distri-cli/src/commands.rs`

- [ ] **Step 1: Remove `handle_config_command` and add `handle_profile_command`**

Remove the old import at the top of `commands.rs`:
```rust
// REMOVE:
use crate::config::set_client_config_value;
use crate::{
    ConfigCommands, ...
};
```

Replace with:
```rust
use crate::{
    ConnectionsCommands, ProfileCommands, ProfileConfigCommands, PromptsCommands,
    SecretsCommands, SkillsCommands, WorkflowCommands, COLOR_BRIGHT_GREEN, COLOR_GRAY, COLOR_RESET,
};
```

Remove `handle_config_command` entirely.

Add at the end of `commands.rs`:

```rust
fn mask_api_key(key: &str) -> String {
    if key.len() > 14 {
        format!("{}...{}", &key[..10], &key[key.len() - 4..])
    } else {
        "***".to_string()
    }
}

pub fn handle_profile_command(command: ProfileCommands) -> anyhow::Result<()> {
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
                let ws_str = values
                    .workspace_id
                    .as_deref()
                    .unwrap_or("(none)");
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
                        "  api_key     = {}",
                        values.api_key.as_deref().map(mask_api_key).unwrap_or_else(|| "(not set)".to_string())
                    );
                    println!(
                        "  workspace_id = {}",
                        values.workspace_id.as_deref().unwrap_or("(not set)")
                    );
                    println!(
                        "  api_url     = {}",
                        values.api_url.as_deref().unwrap_or("https://api.distri.dev/v1 (default)")
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
```

- [ ] **Step 2: Add missing import for `flush` in commands.rs**

Ensure `use std::io::Write;` is already present at the top (it is, from existing code).

- [ ] **Step 3: Verify compile**

```bash
cd distri && cargo check -p distri-cli 2>&1 | grep "^error"
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
cd distri && git add distri-cli/src/commands.rs distri-cli/src/main.rs
git commit -m "feat: add profile list/use/show/delete/config commands"
```

---

### Task 5: Update `login.rs` to save to credentials file with `--profile` support

**Files:**
- Modify: `distri/distri-cli/src/login.rs`

- [ ] **Step 1: Update `handle_login_command` signature and `save_config` call**

In `login.rs`, change the function signature:
```rust
pub async fn handle_login_command(
    _email: Option<String>,
    _skip_workspace: bool,
    profile: Option<String>,
) -> Result<()> {
```

Change the `save_config` call near the end:
```rust
// Replace:
save_config(&api_key, &workspace_id)?;

// With:
let profile_name = profile.as_deref().unwrap_or("default");
save_credentials(profile_name, &api_key, &workspace_id, &login_url_response.login_url)?;
```

Add the print line below it to show which profile was saved:
```rust
println!("\n✓ Successfully authenticated!");
println!("  Profile:      {}", profile_name);
println!(
    "  API Key:      {}...{}",
    &api_key[..10],
    &api_key[api_key.len() - 4..]
);
println!("  Workspace ID: {}", workspace_id);
println!("\nYou can now use 'distri'. Run 'distri -h' for help");
```

- [ ] **Step 2: Replace `save_config` with `save_credentials`**

Remove the old `save_config` and `load_config_toml` functions entirely from `login.rs`.

Add at the bottom of `login.rs`:

```rust
fn save_credentials(
    profile_name: &str,
    api_key: &str,
    workspace_id: &str,
    login_url: &str,
) -> Result<()> {
    // Extract the base api_url from the login_url (strip the /cli-login path)
    let api_url = login_url
        .split("/cli-login")
        .next()
        .unwrap_or("https://api.distri.dev")
        .trim_end_matches('/')
        .to_string()
        + "/v1";

    let values = crate::credentials::ProfileValues {
        api_key: Some(api_key.to_string()),
        workspace_id: Some(workspace_id.to_string()),
        api_url: Some(api_url),
    };
    crate::credentials::save_profile(profile_name, &values)?;
    Ok(())
}
```

- [ ] **Step 3: Remove unused imports in `login.rs`**

Remove these no-longer-needed imports:
```rust
// REMOVE:
use std::path::PathBuf;
use distri::{Distri, DistriConfig};
// Keep:
use distri::Distri;
```

(Keep `DistriConfig` only if still used for `Distri::from_env()`.)

Check what `Distri::from_env()` requires — if it uses `DistriConfig`, keep both. The login command needs a client to call `get_login_url`, which uses `Distri::from_env()`:

```rust
let client = Distri::from_env();
```

`Distri::from_env()` will now call `DistriConfig::from_env()` (unchanged for the SDK). This is fine — login just needs the base_url from the server to get the login page URL, which works with or without credentials.

- [ ] **Step 4: Verify compile**

```bash
cd distri && cargo check -p distri-cli 2>&1 | grep "^error"
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
cd distri && git add distri-cli/src/login.rs
git commit -m "feat: login saves to credentials profile, add --profile flag"
```

---

### Task 6: Remove `set_client_config_value` / `load_client_config_value` from config.rs

**Files:**
- Modify: `distri/distri-cli/src/config.rs`

- [ ] **Step 1: Delete the two dead functions from `config.rs`**

Remove `set_client_config_value` (lines 56–97) and `load_client_config_value` (lines 99–108) entirely from `config.rs`.

The remaining functions to keep: `resolve_workspace`, `get_last_model_file`, `save_last_model`, `load_last_model`, `normalize_optional`, `normalize_base_url`.

- [ ] **Step 2: Verify compile with no dead-code warnings**

```bash
cd distri && cargo check -p distri-cli 2>&1 | grep -E "^error|unused import"
```

Expected: no errors, no unused import warnings for the removed functions.

- [ ] **Step 3: Run full test suite**

```bash
cd distri && cargo test -p distri-cli 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
cd distri && git add distri-cli/src/config.rs
git commit -m "chore: remove deprecated set_client_config_value, all config now via profile"
```

---

### Task 7: Final verification

- [ ] **Step 1: Run clippy**

```bash
cd distri && cargo clippy -p distri-cli -- -D warnings 2>&1 | head -30
```

Fix any warnings before proceeding.

- [ ] **Step 2: Run cargo fmt**

```bash
cd distri && cargo fmt -p distri-cli
```

- [ ] **Step 3: Manual smoke test of profile commands**

```bash
cd distri && cargo run -p distri-cli -- profile list
# Expected: "No profiles found. Run `distri login` or `distri profile config set`..."

cargo run -p distri-cli -- profile config set --api-key dak_test123456 --workspace-id "00000000-0000-0000-0000-000000000001" --api-url "http://localhost:8080/v1"
# Expected: "Updated profile 'default'."

cargo run -p distri-cli -- profile list
# Expected: "* default   api_key=dak_test123...456  workspace=00000000-...  url=http://localhost:8080/v1"

cargo run -p distri-cli -- profile config set --api-key dak_other1234 --profile staging
# Expected: "Updated profile 'staging'."

cargo run -p distri-cli -- profile use staging
# Expected: "Active profile set to 'staging'."

cargo run -p distri-cli -- profile show
# Expected: shows staging profile values

cargo run -p distri-cli -- profile use default
cargo run -p distri-cli -- profile delete staging --yes
# Expected: "Profile 'staging' deleted."
```

- [ ] **Step 4: Verify `distri config` no longer exists**

```bash
cd distri && cargo run -p distri-cli -- config set api_key foo 2>&1
```

Expected: "error: unrecognized subcommand 'config'"

- [ ] **Step 5: Final commit**

```bash
cd distri && cargo fmt -p distri-cli
git add -p
git commit -m "chore: fmt and clippy fixes for cli profiles feature"
```

---

## Self-Review

**Spec coverage:**
- ✅ `~/.distri/credentials` INI format — Task 1
- ✅ `~/.distri/config` active_profile — Task 1 (`get_active_profile`, `set_active_profile`)
- ✅ Legacy migration — Task 1 (`migrate_legacy_config`) + Task 2 (wired in main)
- ✅ `DistriConfig` precedence (env > profile > defaults) — Task 1 (`load_config_with_profile`)
- ✅ `DISTRI_PROFILE` env var — Task 1 (`get_active_profile`)
- ✅ `distri profile list` — Task 4
- ✅ `distri profile use` — Task 4
- ✅ `distri profile show` — Task 4
- ✅ `distri profile delete` — Task 4
- ✅ `distri profile config set` (multi-key, merge) — Task 4
- ✅ `distri profile config unset` — Task 4
- ✅ `distri login --profile` — Task 5
- ✅ `distri config` removed — Task 6
- ✅ Error messages per spec — Task 4

**Placeholder scan:** No TBDs found.

**Type consistency:** `ProfileValues`, `save_profile`, `load_profile`, `unset_profile_keys` defined in Task 1 and used consistently in Tasks 4 and 5.
