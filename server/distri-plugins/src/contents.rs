use anyhow::{anyhow, Result};
use base64::prelude::*;

/// Extract README content from a registry package response
pub async fn extract_readme_from_registry_package(
    package_response: &distri_types::configuration::RegistryPackageResponse,
) -> Result<Option<String>> {
    // Decode the tarball data from base64
    let tarball_data = BASE64_STANDARD
        .decode(&package_response.tarball)
        .map_err(|e| anyhow!("Failed to decode tarball: {}", e))?;

    // For now, since the current registry stores JSON metadata instead of actual tar.gz,
    // try to parse the JSON to extract file information
    if let Ok(metadata_str) = String::from_utf8(tarball_data.clone()) {
        if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(&metadata_str) {
            // Check if this is the metadata format with files array
            if let Some(files) = metadata.get("files").and_then(|f| f.as_array()) {
                // Look for README files in the files list
                let readme_patterns = ["readme", "README"];

                for file_info in files {
                    if let Some(path) = file_info.get("path").and_then(|p| p.as_str()) {
                        let filename = std::path::Path::new(path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("");

                        // Check if this is a README file (case insensitive)
                        for pattern in &readme_patterns {
                            if filename.to_lowercase().starts_with(&pattern.to_lowercase()) {
                                // Extract content if available
                                if let Some(content) =
                                    file_info.get("content").and_then(|c| c.as_str())
                                {
                                    return Ok(Some(content.to_string()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // If we can't extract from metadata, try to extract from the package artifact's tools/agents
    // which might contain description information
    if let Some(config) = package_response.artifact.get("configuration") {
        if let Some(description) = config.get("description").and_then(|d| d.as_str()) {
            if !description.is_empty() {
                // Create a basic README from the package description
                let readme_content = format!(
                    "# {}\n\n{}\n\n**Version:** {}\n",
                    package_response.package, description, package_response.version
                );
                return Ok(Some(readme_content));
            }
        }
    }

    Ok(None)
}
