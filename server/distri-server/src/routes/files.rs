use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use actix_web::{http::header, web, HttpRequest, HttpResponse};
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use distri_core::agent::AgentOrchestrator;
use distri_filesystem::FileSystem;
use distri_types::filesystem::FileSystemOps;
use serde::{Deserialize, Serialize};
use serde_json::json;

const ALLOWED_ROOTS: &[&str] = &["agents", "src", "plugins"];

#[derive(Debug, Serialize, Deserialize, Clone)]
struct WorkspaceFile {
    path: String,
    content: String,
}

#[derive(Debug, Serialize, Clone)]
struct WorkspaceMetadataEntry {
    path: String,
    is_dir: bool,
    size: u64,
    modified: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct WorkspaceMetadataResponse {
    files: Vec<WorkspaceMetadataEntry>,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct WorkspaceSnapshot {
    files: Vec<WorkspaceFile>,
    directories: Vec<String>,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceWriteRequest {
    #[serde(default)]
    files: Vec<WorkspaceFile>,
    #[serde(default)]
    deleted_paths: Vec<String>,
}

#[derive(Debug, Serialize)]
struct WorkspaceWriteResponse {
    saved: usize,
    deleted: usize,
    updated_at: chrono::DateTime<Utc>,
}

pub fn configure_file_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("", web::get().to(list_workspace_files))
        .route("", web::put().to(write_workspace_files))
        .route("/metadata", web::get().to(list_workspace_metadata))
        .service(
            web::resource("/{file_path:.*}")
                .route(web::get().to(read_workspace_file))
                .route(web::head().to(head_workspace_file)),
        );
}

async fn list_workspace_files(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    let filesystem = executor.workspace_filesystem.clone();
    match gather_workspace_snapshot(&filesystem).await {
        Ok(snapshot) => HttpResponse::Ok().json(snapshot),
        Err(err) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to read workspace: {}", err)
        })),
    }
}

async fn write_workspace_files(
    payload: web::Json<WorkspaceWriteRequest>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let request = payload.into_inner();
    let filesystem = executor.workspace_filesystem.clone();

    match persist_workspace_changes(&filesystem, request).await {
        Ok(response) => HttpResponse::Ok().json(response),
        Err(err) => HttpResponse::BadRequest().json(json!({
            "error": err.to_string()
        })),
    }
}

async fn gather_workspace_snapshot(filesystem: &Arc<FileSystem>) -> Result<WorkspaceSnapshot> {
    let WorkspaceTraversal {
        directories,
        files: file_paths,
    } = collect_workspace_paths(filesystem).await?;
    let mut files = Vec::with_capacity(file_paths.len());

    for path in file_paths {
        match read_file_content(filesystem, &path).await {
            Ok(content) => files.push(WorkspaceFile { path, content }),
            Err(err) => {
                tracing::warn!("Skipping unreadable workspace file {}: {}", path, err);
            }
        }
    }

    Ok(WorkspaceSnapshot {
        files,
        directories,
        updated_at: Utc::now(),
    })
}

async fn list_workspace_metadata(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    let filesystem = executor.workspace_filesystem.clone();
    match gather_workspace_metadata(&filesystem).await {
        Ok(files) => HttpResponse::Ok().json(WorkspaceMetadataResponse {
            files,
            updated_at: Utc::now(),
        }),
        Err(err) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to read workspace metadata: {}", err)
        })),
    }
}

async fn gather_workspace_metadata(
    filesystem: &Arc<FileSystem>,
) -> Result<Vec<WorkspaceMetadataEntry>> {
    let WorkspaceTraversal {
        directories,
        files: file_paths,
    } = collect_workspace_paths(filesystem).await?;
    let mut entries = Vec::with_capacity(directories.len() + file_paths.len());
    let directory_timestamp = Utc::now();
    let mut directory_mod_times: HashMap<String, chrono::DateTime<Utc>> = HashMap::new();

    for file_path in file_paths {
        match filesystem.info(&file_path).await {
            Ok(metadata) => {
                let updated = metadata.updated_at;
                entries.push(WorkspaceMetadataEntry {
                    path: file_path.clone(),
                    is_dir: false,
                    size: metadata.size,
                    modified: updated,
                });

                for ancestor in directory_ancestors(&file_path) {
                    directory_mod_times
                        .entry(ancestor)
                        .and_modify(|existing| {
                            if updated > *existing {
                                *existing = updated;
                            }
                        })
                        .or_insert(updated);
                }
            }
            Err(err) => {
                tracing::warn!("Failed to read metadata for {}: {}", file_path, err);
            }
        }
    }

    for directory in directories {
        let modified = directory_mod_times
            .get(&directory)
            .copied()
            .unwrap_or(directory_timestamp);
        entries.push(WorkspaceMetadataEntry {
            path: directory,
            is_dir: true,
            size: 0,
            modified,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

async fn read_workspace_file(
    path: web::Path<String>,
    req: HttpRequest,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let filesystem = executor.workspace_filesystem.clone();
    let relative = path.into_inner();

    let metadata = match workspace_file_metadata(&filesystem, &relative).await {
        Ok(meta) => meta,
        Err(err) => {
            return if is_not_found_error(&err) {
                HttpResponse::NotFound().json(json!({ "error": err.to_string() }))
            } else {
                HttpResponse::BadRequest().json(json!({ "error": err.to_string() }))
            };
        }
    };

    let (sanitized, modified) = metadata;

    if let Some(since) = parse_if_modified_since(&req) {
        if modified <= since {
            return HttpResponse::NotModified().finish();
        }
    }

    match read_file_content(&filesystem, &sanitized).await {
        Ok(content) => HttpResponse::Ok()
            .insert_header((header::LAST_MODIFIED, modified.to_rfc2822()))
            .json(json!({
                "content": content,
                "updated_at": modified,
            })),
        Err(err) => HttpResponse::BadRequest().json(json!({ "error": err.to_string() })),
    }
}

async fn head_workspace_file(
    path: web::Path<String>,
    req: HttpRequest,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let filesystem = executor.workspace_filesystem.clone();
    match workspace_file_metadata(&filesystem, &path.into_inner()).await {
        Ok((_sanitized, modified)) => {
            if let Some(since) = parse_if_modified_since(&req) {
                if modified <= since {
                    return HttpResponse::NotModified().finish();
                }
            }
            HttpResponse::Ok()
                .insert_header((header::LAST_MODIFIED, modified.to_rfc2822()))
                .finish()
        }
        Err(err) => {
            if is_not_found_error(&err) {
                HttpResponse::NotFound().json(json!({ "error": err.to_string() }))
            } else {
                HttpResponse::BadRequest().json(json!({ "error": err.to_string() }))
            }
        }
    }
}

async fn persist_workspace_changes(
    filesystem: &Arc<FileSystem>,
    request: WorkspaceWriteRequest,
) -> Result<WorkspaceWriteResponse> {
    let mut saved = 0usize;
    for file in request.files {
        write_workspace_file(filesystem, file).await?;
        saved += 1;
    }

    let mut deleted = 0usize;
    for path in request.deleted_paths {
        if delete_workspace_path(filesystem, &path).await? {
            deleted += 1;
        }
    }

    Ok(WorkspaceWriteResponse {
        saved,
        deleted,
        updated_at: Utc::now(),
    })
}

async fn write_workspace_file(filesystem: &Arc<FileSystem>, file: WorkspaceFile) -> Result<()> {
    let sanitized = sanitize_workspace_path(&file.path)?;
    filesystem
        .write(&sanitized, &file.content)
        .await
        .with_context(|| format!("Failed to write workspace file {}", file.path))?;
    Ok(())
}

async fn delete_workspace_path(filesystem: &Arc<FileSystem>, relative: &str) -> Result<bool> {
    let sanitized = sanitize_workspace_path(relative)?;
    if filesystem.info(&sanitized).await.is_ok() {
        filesystem
            .delete(&sanitized, false)
            .await
            .with_context(|| format!("Failed to remove workspace file {}", relative))?;
        return Ok(true);
    }

    match filesystem.list(&sanitized).await {
        Ok(listing) => {
            if listing.entries.is_empty() {
                return Ok(false);
            }
            filesystem
                .delete(&sanitized, true)
                .await
                .with_context(|| format!("Failed to remove workspace directory {}", relative))?;
            Ok(true)
        }
        Err(err) => {
            if err.to_string().to_lowercase().contains("not found") {
                Ok(false)
            } else {
                Err(err).context(format!("Failed to inspect workspace path {}", relative))
            }
        }
    }
}

fn sanitize_workspace_path(relative: &str) -> Result<String> {
    if relative.trim().is_empty() {
        return Err(anyhow!("Workspace path cannot be empty"));
    }
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(anyhow!("Absolute paths are not allowed"));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => normalized.push(segment),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow!("Path '{}' escapes the workspace root", relative));
            }
        }
    }

    let mut segments = normalized.iter();
    let first = segments
        .next()
        .ok_or_else(|| anyhow!("Invalid workspace path: {}", relative))?;
    let first_str = first.to_string_lossy();
    if !ALLOWED_ROOTS.iter().any(|allowed| *allowed == first_str) {
        return Err(anyhow!(
            "Path '{}' is outside editable directories ({:?})",
            relative,
            ALLOWED_ROOTS
        ));
    }

    Ok(normalized
        .iter()
        .map(|component| component.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

async fn workspace_file_metadata(
    filesystem: &Arc<FileSystem>,
    relative: &str,
) -> Result<(String, chrono::DateTime<Utc>)> {
    let sanitized = sanitize_workspace_path(relative)?;
    let metadata = match filesystem.info(&sanitized).await {
        Ok(meta) => meta,
        Err(err) => {
            if err.to_string().to_lowercase().contains("not found") {
                return Err(anyhow!("Workspace file not found: {}", relative));
            }
            return Err(anyhow!("Failed to stat {}: {}", relative, err));
        }
    };

    Ok((sanitized, metadata.updated_at))
}

fn directory_ancestors(path: &str) -> Vec<String> {
    let mut ancestors = Vec::new();
    let mut current = PathBuf::from(path);
    while current.pop() {
        if current.as_os_str().is_empty() {
            break;
        }
        ancestors.push(pathbuf_to_string(&current));
    }
    ancestors
}

struct WorkspaceTraversal {
    directories: Vec<String>,
    files: Vec<String>,
}

struct WorkspaceIgnore {
    patterns: Vec<String>,
}

impl WorkspaceIgnore {
    async fn load(filesystem: &Arc<FileSystem>) -> Self {
        let mut patterns = vec![".distri".to_string()];
        if let Ok(bytes) = filesystem.read_binary(".distriignore").await {
            if let Ok(content) = String::from_utf8(bytes) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    patterns.push(trimmed.trim_end_matches('/').to_string());
                }
            }
        }
        patterns.sort();
        patterns.dedup();
        Self { patterns }
    }

    fn should_ignore(&self, path: &str) -> bool {
        let normalized = path.trim_start_matches("./");
        self.patterns
            .iter()
            .any(|pattern| normalized == pattern || normalized.starts_with(&format!("{pattern}/")))
    }
}

async fn collect_workspace_paths(filesystem: &Arc<FileSystem>) -> Result<WorkspaceTraversal> {
    let ignores = WorkspaceIgnore::load(filesystem).await;
    let mut directories = HashSet::new();
    let mut files = Vec::new();

    for relative_root in ALLOWED_ROOTS {
        if ignores.should_ignore(relative_root) {
            continue;
        }

        let sanitized_root = match sanitize_workspace_path(relative_root) {
            Ok(path) => path,
            Err(err) => {
                tracing::warn!("Invalid workspace root {}: {}", relative_root, err);
                continue;
            }
        };

        let mut queue = VecDeque::new();
        queue.push_back(sanitized_root);

        while let Some(current) = queue.pop_front() {
            if ignores.should_ignore(&current) {
                continue;
            }
            if directories.contains(&current) {
                continue;
            }

            match filesystem.list(&current).await {
                Ok(listing) => {
                    directories.insert(current.clone());
                    for entry in listing.entries {
                        let entry_path = join_workspace_paths(&current, &entry.name);
                        if ignores.should_ignore(&entry_path) {
                            continue;
                        }
                        if entry.is_dir {
                            queue.push_back(entry_path);
                        } else if entry.is_file {
                            files.push(entry_path);
                        }
                    }
                }
                Err(err) => {
                    tracing::debug!(
                        "Skipping workspace directory {} due to error: {}",
                        current,
                        err
                    );
                }
            }
        }
    }

    let mut directory_list: Vec<String> = directories.into_iter().collect();
    directory_list.sort();
    files.sort();
    files.dedup();

    Ok(WorkspaceTraversal {
        directories: directory_list,
        files,
    })
}

fn join_workspace_paths(base: &str, segment: &str) -> String {
    if base.is_empty() {
        segment.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches('/'), segment)
    }
}

fn pathbuf_to_string(path: &Path) -> String {
    path.iter()
        .map(|component| component.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

async fn read_file_content(filesystem: &Arc<FileSystem>, path: &str) -> Result<String> {
    let bytes = filesystem
        .read_binary(path)
        .await
        .with_context(|| format!("Failed to read workspace file {}", path))?;
    Ok(String::from_utf8(bytes)
        .with_context(|| format!("Failed to decode workspace file {}", path))?)
}

fn is_not_found_error(err: &anyhow::Error) -> bool {
    let needle = "not found";
    err.chain()
        .any(|cause| cause.to_string().to_lowercase().contains(needle))
}

fn parse_if_modified_since(req: &HttpRequest) -> Option<chrono::DateTime<Utc>> {
    let header = req.headers().get(header::IF_MODIFIED_SINCE)?;
    let value = header.to_str().ok()?;
    chrono::DateTime::parse_from_rfc2822(value)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}
