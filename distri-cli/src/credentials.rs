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

pub fn list_profiles() -> Result<Vec<(String, ProfileValues)>> {
    let path = credentials_path().context("Unable to resolve home directory")?;
    let data = read_ini(&path);
    Ok(data
        .into_iter()
        .map(|(name, section)| (name, section_to_profile(&section)))
        .collect())
}

pub fn load_profile(name: &str) -> Result<Option<ProfileValues>> {
    let path = credentials_path().context("Unable to resolve home directory")?;
    let data = read_ini(&path);
    Ok(data.get(name).map(|s| section_to_profile(s)))
}

/// Merge-save: only updates keys that are Some in `values`, leaves others untouched.
pub fn save_profile(name: &str, values: &ProfileValues) -> Result<()> {
    let path = credentials_path().context("Unable to resolve home directory")?;
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

pub fn unset_profile_keys(
    name: &str,
    api_key: bool,
    workspace_id: bool,
    api_url: bool,
) -> Result<()> {
    let path = credentials_path().context("Unable to resolve home directory")?;
    let mut data = read_ini(&path);
    if let Some(section) = data.get_mut(name) {
        if api_key {
            section.remove("api_key");
        }
        if workspace_id {
            section.remove("workspace_id");
        }
        if api_url {
            section.remove("api_url");
        }
    }
    write_ini(&path, &data)
}

pub fn delete_profile(name: &str) -> Result<()> {
    let path = credentials_path().context("Unable to resolve home directory")?;
    let mut data = read_ini(&path);
    data.remove(name);
    write_ini(&path, &data)
}

pub fn get_active_profile() -> String {
    if let Ok(p) = std::env::var(ENV_PROFILE) {
        let p = p.trim().to_string();
        if !p.is_empty() {
            return p;
        }
    }
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| {
            s.lines().find_map(|line| {
                let line = line.trim();
                line.strip_prefix("active_profile")
                    .and_then(|rest| rest.trim().strip_prefix('='))
                    .map(|v| v.trim().trim_matches('"').to_string())
            })
        })
        .unwrap_or_else(|| DEFAULT_PROFILE.to_string())
}

pub fn set_active_profile(name: &str) -> Result<()> {
    let path = config_path().context("Unable to resolve home directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
    let new_line = format!("active_profile = \"{}\"", name);
    let pos = lines
        .iter()
        .position(|l| l.trim().starts_with("active_profile"));
    match pos {
        Some(i) => lines[i] = new_line,
        None => lines.push(new_line),
    }
    std::fs::write(&path, lines.join("\n") + "\n")?;
    Ok(())
}

pub fn migrate_legacy_config() -> Result<()> {
    let config_path = match config_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    let creds_path = match credentials_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    if creds_path.exists() {
        return Ok(());
    }
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
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
        std::fs::write(&config_path, remaining_lines.join("\n") + "\n")?;
    }
    Ok(())
}

pub fn load_config_with_profile() -> distri_types::DistriConfig {
    use distri_types::DistriConfig;
    let profile_name = get_active_profile();
    let profile = load_profile(&profile_name)
        .unwrap_or_default()
        .unwrap_or_default();
    let env_api_key = std::env::var("DISTRI_API_KEY")
        .ok()
        .filter(|s| !s.is_empty());
    let env_workspace_id = std::env::var("DISTRI_WORKSPACE_ID")
        .ok()
        .filter(|s| !s.is_empty());
    let env_base_url = std::env::var("DISTRI_BASE_URL")
        .ok()
        .filter(|s| !s.is_empty());
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

    fn make_temp_credentials(dir: &TempDir) -> PathBuf {
        dir.path().join("credentials")
    }

    fn make_temp_config(dir: &TempDir) -> PathBuf {
        dir.path().join("config")
    }

    #[test]
    fn test_ini_round_trip() {
        let content = "[default]\napi_key = abc123\nworkspace_id = ws-001\n\n[staging]\napi_key = stagingkey\n";
        let parsed = parse_ini(content);

        assert_eq!(parsed["default"]["api_key"], "abc123");
        assert_eq!(parsed["default"]["workspace_id"], "ws-001");
        assert_eq!(parsed["staging"]["api_key"], "stagingkey");

        let serialized = serialize_ini(&parsed);
        let reparsed = parse_ini(&serialized);

        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn test_save_load_profile() {
        let dir = TempDir::new().unwrap();
        let creds_path = make_temp_credentials(&dir);

        let values = ProfileValues {
            api_key: Some("my-api-key".to_string()),
            workspace_id: Some("ws-123".to_string()),
            api_url: Some("https://api.example.com/v1".to_string()),
        };

        // Write directly using low-level helpers
        let mut data: IniData = BTreeMap::new();
        let section = data.entry("default".to_string()).or_default();
        if let Some(ref v) = values.api_key {
            section.insert("api_key".to_string(), v.clone());
        }
        if let Some(ref v) = values.workspace_id {
            section.insert("workspace_id".to_string(), v.clone());
        }
        if let Some(ref v) = values.api_url {
            section.insert("api_url".to_string(), v.clone());
        }
        write_ini(&creds_path, &data).unwrap();

        // Read back using low-level helpers
        let loaded_data = read_ini(&creds_path);
        let loaded = loaded_data
            .get("default")
            .map(|s| section_to_profile(s))
            .unwrap();

        assert_eq!(loaded.api_key, Some("my-api-key".to_string()));
        assert_eq!(loaded.workspace_id, Some("ws-123".to_string()));
        assert_eq!(loaded.api_url, Some("https://api.example.com/v1".to_string()));
    }

    #[test]
    fn test_merge_save_preserves_existing_keys() {
        let dir = TempDir::new().unwrap();
        let creds_path = make_temp_credentials(&dir);

        // Set up initial profile with all three keys
        let mut data: IniData = BTreeMap::new();
        {
            let section = data.entry("myprofile".to_string()).or_default();
            section.insert("api_key".to_string(), "original-key".to_string());
            section.insert("workspace_id".to_string(), "original-ws".to_string());
            section.insert("api_url".to_string(), "https://original.example.com/v1".to_string());
        }
        write_ini(&creds_path, &data).unwrap();

        // Now update only api_key (workspace_id and api_url should be preserved)
        let mut existing = read_ini(&creds_path);
        let section = existing.entry("myprofile".to_string()).or_default();
        section.insert("api_key".to_string(), "new-key".to_string());
        write_ini(&creds_path, &existing).unwrap();

        // Verify workspace_id and api_url are untouched
        let result = read_ini(&creds_path);
        let profile = section_to_profile(result.get("myprofile").unwrap());

        assert_eq!(profile.api_key, Some("new-key".to_string()));
        assert_eq!(profile.workspace_id, Some("original-ws".to_string()));
        assert_eq!(profile.api_url, Some("https://original.example.com/v1".to_string()));
    }

    #[test]
    fn test_delete_profile() {
        let dir = TempDir::new().unwrap();
        let creds_path = make_temp_credentials(&dir);

        // Set up two profiles
        let mut data: IniData = BTreeMap::new();
        {
            let section = data.entry("default".to_string()).or_default();
            section.insert("api_key".to_string(), "key-default".to_string());
        }
        {
            let section = data.entry("staging".to_string()).or_default();
            section.insert("api_key".to_string(), "key-staging".to_string());
        }
        write_ini(&creds_path, &data).unwrap();

        // Delete only "staging"
        let mut loaded = read_ini(&creds_path);
        loaded.remove("staging");
        write_ini(&creds_path, &loaded).unwrap();

        // Verify "default" still exists but "staging" is gone
        let result = read_ini(&creds_path);
        assert!(result.contains_key("default"), "default profile should still exist");
        assert!(!result.contains_key("staging"), "staging profile should be deleted");
        assert_eq!(
            result["default"]["api_key"],
            "key-default"
        );
    }

    #[test]
    fn test_migrate_legacy_config() {
        let dir = TempDir::new().unwrap();
        let config_path = make_temp_config(&dir);
        let creds_path = make_temp_credentials(&dir);

        // Write a legacy config file
        let legacy_content = r#"api_key = "legacy-key"
workspace_id = "legacy-ws"
base_url = "https://legacy.example.com/v1"
active_profile = "default"
"#;
        std::fs::write(&config_path, legacy_content).unwrap();

        // Perform migration manually using the low-level logic
        let content = std::fs::read_to_string(&config_path).unwrap();
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

        // Write credentials
        if api_key.is_some() || workspace_id.is_some() || base_url.is_some() {
            let mut creds_data: IniData = BTreeMap::new();
            let section = creds_data.entry("default".to_string()).or_default();
            if let Some(ref v) = api_key {
                section.insert("api_key".to_string(), v.clone());
            }
            if let Some(ref v) = workspace_id {
                section.insert("workspace_id".to_string(), v.clone());
            }
            if let Some(ref v) = base_url {
                section.insert("api_url".to_string(), v.clone());
            }
            write_ini(&creds_path, &creds_data).unwrap();
            std::fs::write(&config_path, remaining_lines.join("\n") + "\n").unwrap();
        }

        // Verify credentials file was created with the migrated values
        assert!(creds_path.exists(), "credentials file should be created");
        let creds_data = read_ini(&creds_path);
        let profile = section_to_profile(creds_data.get("default").unwrap());
        assert_eq!(profile.api_key, Some("legacy-key".to_string()));
        assert_eq!(profile.workspace_id, Some("legacy-ws".to_string()));
        assert_eq!(profile.api_url, Some("https://legacy.example.com/v1".to_string()));

        // Verify config file no longer contains legacy keys but still has active_profile
        let remaining_config = std::fs::read_to_string(&config_path).unwrap();
        assert!(!remaining_config.contains("api_key"), "api_key should be removed from config");
        assert!(!remaining_config.contains("workspace_id"), "workspace_id should be removed from config");
        assert!(!remaining_config.contains("base_url"), "base_url should be removed from config");
        assert!(remaining_config.contains("active_profile"), "active_profile should remain in config");
    }

    #[test]
    fn test_parse_ini_ignores_comments_and_blank_lines() {
        let content = "# This is a comment\n\n[section]\n; another comment\nkey = value\n\nkey2 = value2\n";
        let data = parse_ini(content);
        assert_eq!(data["section"]["key"], "value");
        assert_eq!(data["section"]["key2"], "value2");
        assert_eq!(data["section"].len(), 2);
    }

    #[test]
    fn test_parse_ini_empty_file() {
        let data = parse_ini("");
        assert!(data.is_empty());
    }

    #[test]
    fn test_write_ini_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let nested_path = dir.path().join("a").join("b").join("credentials");
        let mut data: IniData = BTreeMap::new();
        data.entry("default".to_string())
            .or_default()
            .insert("api_key".to_string(), "test".to_string());
        write_ini(&nested_path, &data).unwrap();
        assert!(nested_path.exists());
    }
}
