use std::path::{Path, PathBuf};

pub fn resolve_workspace_path() -> PathBuf {
    if let Ok(raw_path) = std::env::var("CURRENT_WORKING_DIR") {
        let resolved = PathBuf::from(raw_path);
        if resolved.is_absolute() {
            return resolved;
        }

        return std::env::current_dir()
            .map(|cwd| cwd.join(resolved))
            .unwrap_or_else(|_| PathBuf::from("."));
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let examples_dir = cwd.join("examples");
    if examples_dir.is_dir() {
        examples_dir
    } else {
        cwd
    }
}

pub fn ensure_workspace_scaffold(root: &Path) -> std::io::Result<()> {
    for entry in ["agents", "src", "plugins"] {
        if !root.join(entry).exists() {
            std::fs::create_dir_all(root.join(entry))?;
        }
    }
    Ok(())
}
