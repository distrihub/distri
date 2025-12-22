use std::sync::Arc;

use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_filesystem::ArtifactWrapper;
use distri_types::filesystem::FileSystemOps;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Configure artifact routes under `/artifacts`
/// 
/// The artifact_id is a flexible namespace identifier that can be:
/// - A task namespace: `threads/{thread_hash}/tasks/{task_hash}` (computed from thread_id + task_id)
/// - A shared space: `shared/{space_name}`
/// - Any other namespace structure
/// 
/// Routes:
/// - GET /artifacts - List all accessible artifact namespaces
/// - GET /artifacts/{artifact_id...} - List artifacts in a namespace
/// - GET /artifacts/{artifact_id...}/content/{filename} - Read artifact content
/// - PUT /artifacts/{artifact_id...}/content/{filename} - Save artifact
/// - DELETE /artifacts/{artifact_id...}/content/{filename} - Delete artifact
/// - POST /artifacts/{artifact_id...}/search - Search within artifacts
pub fn configure_artifact_routes(cfg: &mut web::ServiceConfig) {
    cfg
        // List all accessible artifact namespaces
        .route("", web::get().to(list_all_namespaces))
        // Compute task namespace from thread_id and task_id (convenience endpoint)
        .route("/task/{thread_id}/{task_id}", web::get().to(get_task_namespace))
        // Operations on a specific namespace (artifact_id is the full path like "threads/abc/tasks/def")
        .service(
            web::resource("/{artifact_id:.*}/content/{filename}")
                .route(web::get().to(read_artifact))
                .route(web::put().to(save_artifact))
                .route(web::delete().to(delete_artifact)),
        )
        .service(
            web::resource("/{artifact_id:.*}/search")
                .route(web::post().to(search_artifacts)),
        )
        // List artifacts in a namespace (must come last due to catch-all pattern)
        .service(
            web::resource("/{artifact_id:.*}")
                .route(web::get().to(list_artifacts)),
        );
}

// ─────────────────────────────────────────────────────────────────────────────
// Response Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct NamespaceListResponse {
    namespaces: Vec<ArtifactNamespace>,
}

#[derive(Debug, Serialize)]
struct ArtifactNamespace {
    /// The artifact_id / namespace path (e.g., "threads/abc123/tasks/def456")
    artifact_id: String,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    /// Number of artifacts in this namespace (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact_count: Option<usize>,
}

#[derive(Debug, Serialize)]
struct TaskNamespaceResponse {
    /// The computed artifact_id for the task
    artifact_id: String,
    /// Original thread_id
    thread_id: String,
    /// Original task_id  
    task_id: String,
}

#[derive(Debug, Serialize)]
struct ArtifactListResponse {
    /// The artifact namespace
    artifact_id: String,
    /// List of artifacts in this namespace
    artifacts: Vec<ArtifactEntry>,
    /// Full path to content directory
    content_path: String,
}

#[derive(Debug, Serialize)]
struct ArtifactEntry {
    /// Just the filename (e.g., "data.json")
    filename: String,
    /// Whether this is a file
    is_file: bool,
    /// File size in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    /// Full path to read this artifact
    read_path: String,
}

#[derive(Debug, Serialize)]
struct ReadArtifactResponse {
    content: String,
    start_line: u64,
    end_line: u64,
    total_lines: u64,
    filename: String,
    artifact_id: String,
}

#[derive(Debug, Serialize)]
struct SaveArtifactResponse {
    success: bool,
    filename: String,
    artifact_id: String,
    size: usize,
}

#[derive(Debug, Deserialize)]
struct ReadArtifactQuery {
    start_line: Option<u64>,
    end_line: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct SaveArtifactRequest {
    content: String,
}

#[derive(Debug, Deserialize)]
struct SearchArtifactsRequest {
    pattern: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers
// ─────────────────────────────────────────────────────────────────────────────

/// List all accessible artifact namespaces
async fn list_all_namespaces(
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let filesystem = executor.session_filesystem.clone();
    
    // List top-level directories to find namespaces
    let mut namespaces = Vec::new();
    
    // Check for "threads" directory (task-based artifacts)
    if let Ok(threads_listing) = filesystem.list("threads").await {
        for thread_entry in threads_listing.entries {
            if thread_entry.is_dir {
                let thread_path = format!("threads/{}", thread_entry.name);
                
                // Check for "tasks" subdirectory
                if let Ok(tasks_dir) = filesystem.list(&format!("{}/tasks", thread_path)).await {
                    for task_entry in tasks_dir.entries {
                        if task_entry.is_dir {
                            let artifact_id = format!("{}/tasks/{}", thread_path, task_entry.name);
                            
                            // Count artifacts in content directory
                            let artifact_count = filesystem
                                .list(&format!("{}/content", artifact_id))
                                .await
                                .map(|l| l.entries.len())
                                .ok();
                            
                            namespaces.push(ArtifactNamespace {
                                artifact_id,
                                description: Some(format!("Task artifacts for thread {}", thread_entry.name)),
                                artifact_count,
                            });
                        }
                    }
                }
            }
        }
    }
    
    // Check for "shared" directory (shared artifacts)
    if let Ok(shared_listing) = filesystem.list("shared").await {
        for entry in shared_listing.entries {
            if entry.is_dir {
                let artifact_id = format!("shared/{}", entry.name);
                
                let artifact_count = filesystem
                    .list(&format!("{}/content", artifact_id))
                    .await
                    .map(|l| l.entries.len())
                    .ok();
                
                namespaces.push(ArtifactNamespace {
                    artifact_id,
                    description: Some(format!("Shared space: {}", entry.name)),
                    artifact_count,
                });
            }
        }
    }
    
    HttpResponse::Ok().json(NamespaceListResponse { namespaces })
}

/// Get the computed artifact_id (namespace) for a thread/task pair
async fn get_task_namespace(
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (thread_id, task_id) = path.into_inner();
    let artifact_id = ArtifactWrapper::task_namespace(&thread_id, &task_id);
    
    HttpResponse::Ok().json(TaskNamespaceResponse {
        artifact_id,
        thread_id,
        task_id,
    })
}

/// List all artifacts in a namespace
async fn list_artifacts(
    path: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let artifact_id = path.into_inner();
    
    // Handle empty path (should use list_all_namespaces instead)
    if artifact_id.is_empty() {
        return list_all_namespaces(executor).await;
    }
    
    let filesystem = executor.session_filesystem.clone();
    
    // Use ArtifactWrapper helper to list artifacts from all paths (thread and task level)
    let paths_to_check = ArtifactWrapper::get_paths_to_check(&artifact_id);
    let mut all_artifacts: Vec<ArtifactEntry> = Vec::new();
    let mut seen_filenames = std::collections::HashSet::new();
    
    // Check each path and track which path each artifact came from
    for path_id in paths_to_check {
        if let Ok(wrapper) = ArtifactWrapper::new(
            filesystem.clone() as Arc<dyn FileSystemOps>,
            path_id.clone(),
        ).await {
            if let Ok(entries) = wrapper.list_artifacts().await {
                for e in entries {
                    if !seen_filenames.contains(&e.name) {
                        seen_filenames.insert(e.name.clone());
                        // Use the actual path_id where this artifact was found
                        all_artifacts.push(ArtifactEntry {
                            filename: e.name.clone(),
                            is_file: e.is_file,
                            size: e.size,
                            read_path: format!("/artifacts/{}/content/{}", path_id, e.name),
                        });
                    }
                }
            }
        }
    }
    
    HttpResponse::Ok().json(ArtifactListResponse {
        artifact_id: artifact_id.clone(),
        artifacts: all_artifacts,
        content_path: format!("{}/content", artifact_id),
    })
}

/// Read a specific artifact
async fn read_artifact(
    path: web::Path<(String, String)>,
    query: web::Query<ReadArtifactQuery>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let (artifact_id, filename) = path.into_inner();
    
    let filesystem = executor.session_filesystem.clone();
    
    // Use ArtifactWrapper helper to read artifact from all paths (thread and task level)
    match ArtifactWrapper::read_artifact_multi_path(
        filesystem.clone() as Arc<dyn FileSystemOps>,
        &artifact_id,
        &filename,
        query.start_line,
        query.end_line,
    ).await {
        Ok((result, path_id)) => {
            // Found it! Return the result with the full relative path (threads/{hash}/content/{filename})
            let full_artifact_path = format!("{}/content/{}", path_id, filename);
            HttpResponse::Ok().json(ReadArtifactResponse {
                content: result.content,
                start_line: result.start_line,
                end_line: result.end_line,
                total_lines: result.total_lines,
                filename,
                artifact_id: full_artifact_path, // Full relative path like threads/{hash}/content/{filename}
            })
        }
        Err(e) => {
            if e.to_string().to_lowercase().contains("not found") {
                HttpResponse::NotFound().json(json!({
                    "error": format!("Artifact not found: {}", filename),
                    "artifact_id": artifact_id
                }))
            } else {
                HttpResponse::InternalServerError().json(json!({
                    "error": format!("Failed to read artifact: {}", e)
                }))
            }
        }
    }
}

/// Save an artifact
async fn save_artifact(
    path: web::Path<(String, String)>,
    body: web::Json<SaveArtifactRequest>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let (artifact_id, filename) = path.into_inner();
    
    let filesystem = executor.session_filesystem.clone();
    
    // Get the filesystem root prefix to show absolute path (before moving filesystem)
    let filesystem_root = filesystem.root_prefix().unwrap_or_else(|| "".to_string());
    
    let wrapper = match ArtifactWrapper::new(
        filesystem.clone() as Arc<dyn FileSystemOps>,
        artifact_id.clone(),
    ).await {
        Ok(w) => w,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "error": format!("Failed to create artifact wrapper: {}", e)
            }));
        }
    };
    
    let artifact_path = format!("{}/content/{}", artifact_id, filename);
    
    // Calculate the full path within the object store
    // filesystem_root is the prefix within the object store (e.g., "runs/default")
    let store_path = if filesystem_root.is_empty() {
        artifact_path.clone()
    } else {
        format!("{}/{}", filesystem_root, artifact_path)
    };
    
    // For local filesystem, try to get the actual absolute path on disk
    // The base_path is in the object store config, but we can construct it
    // Default base_path is ".distri/files" per FileSystemConfig::default()
    // We'll show both the store path and attempt to show where it would be on disk
    let base_path = ".distri/files"; // Default from FileSystemConfig
    let absolute_path = std::path::Path::new(base_path)
        .join(&store_path)
        .canonicalize()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| {
            // If canonicalize fails (path doesn't exist yet), construct the expected path
            std::path::Path::new(base_path)
                .join(&store_path)
                .to_string_lossy()
                .to_string()
        });
    
    tracing::info!(
        "Saving artifact: artifact_id={}, filename={}, relative_path={}, store_path={}, absolute_path={}, size={} bytes",
        artifact_id,
        filename,
        artifact_path,
        store_path,
        absolute_path,
        body.content.len()
    );
    
    match wrapper.save_artifact(&filename, &body.content).await {
        Ok(()) => {
            // Log the full path that was used - try to get the actual canonical path after saving
            let final_absolute_path = std::path::Path::new(base_path)
                .join(&store_path)
                .canonicalize()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| absolute_path.clone());
            
            tracing::info!(
                "Successfully saved artifact: relative_path={}, store_path={}, absolute_path={} (artifact_id: {}, filename: {})",
                artifact_path,
                store_path,
                final_absolute_path,
                artifact_id,
                filename
            );
            HttpResponse::Ok().json(SaveArtifactResponse {
                success: true,
                filename,
                artifact_id,
                size: body.content.len(),
            })
        }
        Err(e) => {
            tracing::error!(
                "Failed to save artifact at path {}: {}",
                artifact_path,
                e
            );
            HttpResponse::InternalServerError().json(json!({
                "error": format!("Failed to save artifact: {}", e),
                "path": artifact_path
            }))
        }
    }
}

/// Delete an artifact (cleans up the namespace folder)
async fn delete_artifact(
    path: web::Path<(String, String)>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let (artifact_id, _filename) = path.into_inner();
    
    let filesystem = executor.session_filesystem.clone();
    
    let wrapper = match ArtifactWrapper::new(
        filesystem as Arc<dyn FileSystemOps>,
        artifact_id.clone(),
    ).await {
        Ok(w) => w,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "error": format!("Failed to create artifact wrapper: {}", e)
            }));
        }
    };
    
    match wrapper.cleanup_task_folder().await {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(e) => {
            HttpResponse::InternalServerError().json(json!({
                "error": format!("Failed to delete artifact: {}", e)
            }))
        }
    }
}

/// Search within artifacts
async fn search_artifacts(
    path: web::Path<String>,
    body: web::Json<SearchArtifactsRequest>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let artifact_id = path.into_inner();
    
    let filesystem = executor.session_filesystem.clone();
    
    let wrapper = match ArtifactWrapper::new(
        filesystem as Arc<dyn FileSystemOps>,
        artifact_id.clone(),
    ).await {
        Ok(w) => w,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "error": format!("Failed to create artifact wrapper: {}", e)
            }));
        }
    };
    
    match wrapper.search_artifacts(&body.pattern).await {
        Ok(result) => HttpResponse::Ok().json(json!({
            "artifact_id": artifact_id,
            "pattern": body.pattern,
            "results": result
        })),
        Err(e) => {
            HttpResponse::InternalServerError().json(json!({
                "error": format!("Failed to search artifacts: {}", e)
            }))
        }
    }
}
