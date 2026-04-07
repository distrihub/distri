use std::path::PathBuf;

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

