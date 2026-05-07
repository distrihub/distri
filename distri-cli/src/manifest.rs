use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub server: Option<EntryRecord>,
    pub ui: Option<EntryRecord>,
    pub releases_index_etag: Option<String>,
    pub releases_index_fetched_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EntryRecord {
    pub version: String,
    pub installed_at: DateTime<Utc>,
    pub sha256: String,
    pub path: PathBuf,
}

pub fn distri_home() -> Result<PathBuf> {
    let base = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not resolve home dir"))?;
    Ok(base.join(".distri"))
}

pub fn manifest_path() -> Result<PathBuf> {
    Ok(distri_home()?.join("cache").join("manifest.json"))
}

pub fn read() -> Result<Manifest> {
    let p = manifest_path()?;
    if !p.exists() {
        return Ok(Manifest::default());
    }
    let s = std::fs::read_to_string(&p)?;
    Ok(serde_json::from_str(&s).unwrap_or_default())
}

pub fn write(m: &Manifest) -> Result<()> {
    let p = manifest_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let s = serde_json::to_string_pretty(m)?;
    std::fs::write(p, s)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn with_temp_home<F: FnOnce()>(f: F) {
        let tmp = TempDir::new().unwrap();
        // Hold the env-var setter in scope so concurrent tests don't fight over it.
        // Vitest-style isolation isn't great in cargo test for env, so we run
        // single-threaded by accepting the trade-off and only setting HOME inside
        // the closure.
        std::env::set_var("HOME", tmp.path());
        f();
    }

    #[test]
    fn read_returns_default_when_missing() {
        with_temp_home(|| {
            let m = read().expect("read default");
            assert!(m.server.is_none());
            assert!(m.ui.is_none());
            assert!(m.releases_index_etag.is_none());
        });
    }

    #[test]
    fn round_trip_preserves_fields() {
        with_temp_home(|| {
            let m = Manifest {
                server: Some(EntryRecord {
                    version: "0.5.3".into(),
                    installed_at: Utc::now(),
                    sha256: "deadbeef".into(),
                    path: PathBuf::from("/tmp/distri-server"),
                }),
                ui: Some(EntryRecord {
                    version: "0.5.7".into(),
                    installed_at: Utc::now(),
                    sha256: "cafef00d".into(),
                    path: PathBuf::from("/tmp/distri-ui"),
                }),
                releases_index_etag: Some("W/\"abc\"".into()),
                releases_index_fetched_at: Some(Utc::now()),
            };
            write(&m).expect("write");
            let r = read().expect("read");
            assert_eq!(r.server.as_ref().unwrap().version, "0.5.3");
            assert_eq!(r.ui.as_ref().unwrap().version, "0.5.7");
            assert_eq!(r.releases_index_etag.as_deref(), Some("W/\"abc\""));
        });
    }

    #[test]
    fn write_creates_parent_dirs() {
        with_temp_home(|| {
            let p = manifest_path().unwrap();
            assert!(!p.exists());
            write(&Manifest::default()).unwrap();
            assert!(p.exists());
        });
    }

    #[test]
    fn corrupt_manifest_returns_default() {
        with_temp_home(|| {
            let p = manifest_path().unwrap();
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "this is not json").unwrap();
            let m = read().unwrap();
            assert!(m.server.is_none());
            assert!(m.ui.is_none());
        });
    }
}
