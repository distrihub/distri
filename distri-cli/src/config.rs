use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use distri::DistriConfig;

pub fn resolve_workspace(config_path: &Option<PathBuf>) -> PathBuf {
    config_path
        .as_ref()
        .and_then(|path| path.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn get_last_model_file() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".distri").join("last_model")
}

pub fn save_last_model(model: Option<&str>) {
    let path = get_last_model_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match model {
        Some(m) => {
            let _ = std::fs::write(&path, m);
        }
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }
}

pub fn load_last_model() -> Option<String> {
    let path = get_last_model_file();
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn normalize_optional(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn normalize_base_url(raw: &str) -> Option<String> {
    normalize_optional(raw).map(|value| value.trim_end_matches('/').to_string())
}

pub fn set_client_config_value(key: &str, raw_value: &str) -> Result<PathBuf> {
    let path = DistriConfig::config_path()
        .ok_or_else(|| anyhow::anyhow!("Unable to resolve home directory for ~/.distri/config"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config = load_client_config_value(&path);
    let normalized = match key {
        "api_key" => normalize_optional(raw_value),
        "base_url" => normalize_base_url(raw_value),
        "workspace_id" => {
            let trimmed = normalize_optional(raw_value);
            if let Some(ref value) = trimmed {
                // Validate that it's a valid UUID
                uuid::Uuid::parse_str(value).with_context(|| {
                    format!("Invalid workspace_id: '{}' is not a valid UUID", value)
                })?;
            }
            trimmed
        }
        _ => anyhow::bail!(
            "Unknown config key '{}'. Supported keys: api_key, base_url, workspace_id",
            key
        ),
    };

    if let toml::Value::Table(ref mut table) = config {
        match normalized {
            Some(value) => {
                table.insert(key.to_string(), toml::Value::String(value));
            }
            None => {
                table.remove(key);
            }
        }
    }

    let contents = toml::to_string_pretty(&config)?;
    std::fs::write(&path, contents)?;
    Ok(path)
}

pub fn load_client_config_value(path: &Path) -> toml::Value {
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| contents.parse::<toml::Value>().ok());

    match parsed {
        Some(toml::Value::Table(table)) => toml::Value::Table(table),
        _ => toml::Value::Table(toml::map::Map::new()),
    }
}
