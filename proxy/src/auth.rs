use crate::types::ProxyMcpServer;
use crate::types::ProxyTransportAuth;
use std::collections::HashMap;

const DEFAULT_CREDENTIALS_DIR: &str = "./creds";

pub(crate) fn handle_auth(
    server: &ProxyMcpServer,
    client_key: &str,
    server_env_vars: Option<&HashMap<String, String>>,
) -> HashMap<String, String> {
    let mut new_vars = HashMap::new();
    if let Some(ProxyTransportAuth::Files(files)) = &server.auth {
        let hash_dir = format!("{DEFAULT_CREDENTIALS_DIR}/{client_key}");
        std::fs::create_dir_all(&hash_dir).unwrap_or_else(|e| {
            tracing::warn!("Failed to create {} directory: {}", hash_dir, e);
        });

        new_vars.insert("creds_directory".to_string(), hash_dir.clone());

        for (key, value) in files.iter() {
            // Write the value to a file
            let file_path = format!("{}/{}", hash_dir, key);
            let mut file_value = serde_json::to_string_pretty(value).unwrap();
            if let Some(env_vars) = server_env_vars {
                for (key, value) in env_vars {
                    if key == "expiry_date" {
                        file_value = file_value.replace(&format!("\"{{{{{key}}}}}\""), value);
                    } else {
                        file_value = file_value.replace(&format!("{{{{{key}}}}}"), value);
                    }
                }
            }

            match std::fs::write(&file_path, file_value) {
                Ok(_) => {
                    tracing::info!("Created file: {}", file_path);
                }
                Err(e) => {
                    tracing::error!("Failed to write file {}: {}", file_path, e);
                }
            }
        }
    }

    new_vars
}
