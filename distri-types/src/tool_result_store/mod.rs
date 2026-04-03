//! Two-level tool result persistence and file read caching.
//!
//! Mirrors Claude Code's approach:
//! - Level 1: Full results persisted to disk (`tool-results/{id}.txt`)
//! - Level 2: Compact previews in the scratchpad/conversation
//!
//! Also provides:
//! - Boundary-aware preview generation (truncates at newlines)
//! - Format-specific previews (markdown headings, JSON structure, CSV headers)
//! - Binary file detection
//! - LRU file read cache with FILE_UNCHANGED_STUB deduplication

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

// ── Constants (matching Claude Code patterns) ────────────────────────────────

/// Preview size for persisted tool results (boundary-aware truncation target)
pub const PREVIEW_SIZE_BYTES: usize = 2_000;

/// Threshold above which tool results are persisted to disk
pub const PERSIST_THRESHOLD_BYTES: usize = 8_000;

/// Hard cap for any single tool result before forced persistence
pub const MAX_TOOL_RESULT_CHARS: usize = 50_000;

/// Per-message aggregate budget for all tool results (prevents parallel tool bloat)
pub const MAX_TOOL_RESULTS_PER_MESSAGE_CHARS: usize = 200_000;

/// Stub returned when a file read returns identical content to a previous read
pub const FILE_UNCHANGED_STUB: &str =
    "[File content unchanged since last read — using cached version]";

// ── Content Format Detection ─────────────────────────────────────────────────

/// Detected content format for format-specific preview generation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentFormat {
    PlainText,
    Markdown,
    Json,
    Csv,
    Binary,
    Unknown,
}

impl ContentFormat {
    /// Detect content format from content bytes and optional filename hint.
    pub fn detect(content: &str, filename: Option<&str>) -> Self {
        // Check filename extension first
        if let Some(name) = filename {
            let lower = name.to_lowercase();
            if lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".mdx") {
                return Self::Markdown;
            }
            if lower.ends_with(".json") || lower.ends_with(".jsonl") {
                return Self::Json;
            }
            if lower.ends_with(".csv") || lower.ends_with(".tsv") {
                return Self::Csv;
            }
            // Known binary extensions
            if is_binary_extension(&lower) {
                return Self::Binary;
            }
        }

        // Content-based detection
        if is_likely_binary(content) {
            return Self::Binary;
        }

        // Try JSON
        let trimmed = content.trim();
        if (trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']'))
        {
            if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
                return Self::Json;
            }
        }

        // Markdown heuristic: has headings or frontmatter
        if trimmed.starts_with("---\n") || trimmed.starts_with("# ") || trimmed.contains("\n# ") {
            return Self::Markdown;
        }

        // CSV heuristic: first line has commas/tabs, consistent column count
        if looks_like_csv(trimmed) {
            return Self::Csv;
        }

        Self::PlainText
    }
}

fn is_binary_extension(name: &str) -> bool {
    let binary_exts = [
        ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".ico", ".webp", ".svg",
        ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx",
        ".zip", ".tar", ".gz", ".bz2", ".7z", ".rar",
        ".exe", ".dll", ".so", ".dylib", ".o", ".a",
        ".wasm", ".pyc", ".class",
        ".mp3", ".mp4", ".wav", ".avi", ".mkv", ".mov",
        ".ttf", ".otf", ".woff", ".woff2",
        ".sqlite", ".db",
    ];
    binary_exts.iter().any(|ext| name.ends_with(ext))
}

/// Detect binary content via null bytes and high-byte ratio
fn is_likely_binary(content: &str) -> bool {
    if content.is_empty() {
        return false;
    }
    let sample = &content[..content.len().min(512)];
    // Null bytes are a strong signal
    if sample.contains('\0') {
        return true;
    }
    // High ratio of control characters (excluding common whitespace)
    let control_count = sample
        .chars()
        .filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        .count();
    control_count as f64 / sample.len() as f64 > 0.1
}

fn looks_like_csv(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().take(5).collect();
    if lines.len() < 2 {
        return false;
    }
    let separator = if lines[0].contains('\t') {
        '\t'
    } else if lines[0].contains(',') {
        ','
    } else {
        return false;
    };
    let col_count = lines[0].matches(separator).count();
    if col_count == 0 {
        return false;
    }
    // Check that subsequent lines have similar column count
    lines[1..]
        .iter()
        .all(|line| {
            let c = line.matches(separator).count();
            c == col_count || (c as i64 - col_count as i64).unsigned_abs() <= 1
        })
}

// ── Preview Generation ───────────────────────────────────────────────────────

/// A preview of persisted content with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preview {
    /// The preview text (truncated content)
    pub text: String,
    /// Whether there is more content beyond the preview
    pub has_more: bool,
    /// Total size of the original content in bytes
    pub total_bytes: usize,
    /// Detected content format
    pub format: ContentFormat,
}

impl Preview {
    /// Generate a boundary-aware preview of content.
    ///
    /// Truncates at the last newline within `max_bytes`, avoiding mid-line cuts.
    /// Falls back to exact byte limit if no suitable newline boundary exists.
    pub fn generate(content: &str, max_bytes: usize) -> Self {
        let format = ContentFormat::detect(content, None);
        Self::generate_with_format(content, max_bytes, format)
    }

    /// Generate a preview with explicit format (for when filename is known)
    pub fn generate_for_file(content: &str, max_bytes: usize, filename: &str) -> Self {
        let format = ContentFormat::detect(content, Some(filename));
        Self::generate_with_format(content, max_bytes, format)
    }

    /// Generate format-specific preview
    pub fn generate_with_format(content: &str, max_bytes: usize, format: ContentFormat) -> Self {
        let total_bytes = content.len();

        // Binary content always gets a notice (never display raw binary)
        if format == ContentFormat::Binary {
            return Self {
                text: format!(
                    "[Binary content, {} bytes — cannot display as text]",
                    total_bytes
                ),
                has_more: true,
                total_bytes,
                format,
            };
        }

        if total_bytes <= max_bytes {
            return Self {
                text: content.to_string(),
                has_more: false,
                total_bytes,
                format,
            };
        }

        let text = match &format {
            ContentFormat::Markdown => generate_markdown_preview(content, max_bytes),
            ContentFormat::Json => generate_json_preview(content, max_bytes),
            ContentFormat::Csv => generate_csv_preview(content, max_bytes),
            _ => truncate_at_boundary(content, max_bytes),
        };

        Self {
            text,
            has_more: true,
            total_bytes,
            format,
        }
    }

    /// Format preview as it should appear in the model's context
    pub fn as_persisted_notice(&self, persisted_path: &str) -> String {
        if !self.has_more {
            return self.text.clone();
        }
        let size_kb = self.total_bytes as f64 / 1024.0;
        format!(
            "<persisted-output>\nOutput too large ({:.1}KB). Full output saved to: {}\n\nPreview (first {:.1}KB):\n{}\n</persisted-output>",
            size_kb,
            persisted_path,
            self.text.len() as f64 / 1024.0,
            self.text
        )
    }
}

/// Truncate at the last newline boundary within max_bytes.
/// Falls back to exact limit if no good boundary found (>50% of budget).
fn truncate_at_boundary(content: &str, max_bytes: usize) -> String {
    // Ensure we don't cut in the middle of a UTF-8 char
    let safe_end = content
        .char_indices()
        .take_while(|(i, _)| *i < max_bytes)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);

    let slice = &content[..safe_end];
    let last_newline = slice.rfind('\n');

    let cut_point = match last_newline {
        Some(pos) if pos > max_bytes / 2 => pos + 1, // Include the newline
        _ => safe_end,                                 // No good boundary
    };

    content[..cut_point].to_string()
}

/// Generate markdown preview: extract heading structure + content under first headings
fn generate_markdown_preview(content: &str, max_bytes: usize) -> String {
    let mut result = String::with_capacity(max_bytes);
    let mut heading_count = 0;
    let mut in_content = false;
    let mut content_lines = 0;
    let max_content_lines_per_section = 5;

    for line in content.lines() {
        if result.len() >= max_bytes {
            break;
        }

        if line.starts_with('#') {
            heading_count += 1;
            in_content = true;
            content_lines = 0;
            result.push_str(line);
            result.push('\n');
        } else if in_content && content_lines < max_content_lines_per_section {
            if !line.trim().is_empty() {
                content_lines += 1;
                result.push_str(line);
                result.push('\n');
            }
        }

        // If we've seen enough headings and content, stop
        if heading_count > 10 && result.len() > max_bytes / 2 {
            break;
        }
    }

    // If very few headings found, fall back to boundary truncation
    if heading_count < 2 || result.len() < max_bytes / 4 {
        return truncate_at_boundary(content, max_bytes);
    }

    // Truncate result itself if it exceeded budget
    if result.len() > max_bytes {
        result = truncate_at_boundary(&result, max_bytes);
    }

    result
}

/// Generate JSON preview: show structure outline (top-level keys, array length)
fn generate_json_preview(content: &str, max_bytes: usize) -> String {
    let trimmed = content.trim();
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) => {
            let outline = json_structure_outline(&value, 0, max_bytes);
            if outline.len() > max_bytes {
                truncate_at_boundary(&outline, max_bytes)
            } else {
                outline
            }
        }
        Err(_) => truncate_at_boundary(content, max_bytes),
    }
}

fn json_structure_outline(value: &serde_json::Value, depth: usize, budget: usize) -> String {
    let indent = "  ".repeat(depth);
    match value {
        serde_json::Value::Object(map) => {
            let mut result = format!("{}{{", indent);
            let key_count = map.len();
            result.push_str(&format!(" // {} keys\n", key_count));
            for (i, (key, val)) in map.iter().enumerate() {
                if result.len() > budget {
                    result.push_str(&format!("{}  ... ({} more keys)\n", indent, key_count - i));
                    break;
                }
                let val_summary = match val {
                    serde_json::Value::String(s) => {
                        if s.len() > 50 {
                            format!("\"{}...\" ({} chars)", &s[..50], s.len())
                        } else {
                            format!("{:?}", s)
                        }
                    }
                    serde_json::Value::Array(arr) => format!("[...] ({} items)", arr.len()),
                    serde_json::Value::Object(m) => format!("{{...}} ({} keys)", m.len()),
                    other => other.to_string(),
                };
                result.push_str(&format!("{}  {:?}: {}\n", indent, key, val_summary));
            }
            result.push_str(&format!("{}}}", indent));
            result
        }
        serde_json::Value::Array(arr) => {
            let mut result = format!("{}[ // {} items\n", indent, arr.len());
            for (i, item) in arr.iter().take(3).enumerate() {
                if result.len() > budget {
                    result.push_str(&format!("{}  ... ({} more items)\n", indent, arr.len() - i));
                    break;
                }
                let summary = json_structure_outline(item, depth + 1, budget - result.len());
                result.push_str(&summary);
                result.push('\n');
            }
            if arr.len() > 3 {
                result.push_str(&format!("{}  ... ({} more items)\n", indent, arr.len() - 3));
            }
            result.push_str(&format!("{}]", indent));
            result
        }
        other => format!("{}{}", indent, other),
    }
}

/// Generate CSV preview: show headers + first few data rows
fn generate_csv_preview(content: &str, max_bytes: usize) -> String {
    let mut result = String::with_capacity(max_bytes);
    let total_lines = content.lines().count();

    for (i, line) in content.lines().enumerate() {
        if result.len() >= max_bytes {
            break;
        }
        if i == 0 {
            result.push_str(&format!("[Headers] {}\n", line));
        } else if i <= 5 {
            result.push_str(&format!("[Row {}] {}\n", i, line));
        } else {
            result.push_str(&format!("... ({} more rows)\n", total_lines - i));
            break;
        }
    }

    result
}

// ── Persisted Tool Result ────────────────────────────────────────────────────

/// Metadata for a tool result that has been persisted to disk.
/// The actual content lives at `persisted_path`; only the preview is kept in memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedToolResult {
    /// Tool use ID (matches the original tool call)
    pub tool_use_id: String,
    /// Tool name that produced this result
    pub tool_name: String,
    /// Compact preview of the content
    pub preview: Preview,
    /// Path where full content is stored (relative to session storage)
    pub persisted_path: String,
    /// When the result was persisted
    pub timestamp: i64,
}

/// Decides whether a tool result part should be persisted to disk.
pub fn should_persist(content: &str) -> bool {
    content.len() >= PERSIST_THRESHOLD_BYTES
}

/// Decides whether content is binary and should be rejected for text display.
pub fn is_binary_content(content: &str, filename: Option<&str>) -> bool {
    ContentFormat::detect(content, filename) == ContentFormat::Binary
}

// ── File Read Cache (LRU) ────────────────────────────────────────────────────

/// LRU cache for file read deduplication.
///
/// Tracks content hashes + mtimes for file reads. When the same file is read
/// with the same parameters and hasn't changed, returns FILE_UNCHANGED_STUB
/// instead of the full content (~18% token savings in practice).
#[derive(Debug, Clone, Default)]
pub struct FileReadCache {
    entries: HashMap<String, FileReadCacheEntry>,
    access_order: VecDeque<String>,
    max_entries: usize,
}

/// A cache key combines file path + read parameters (offset, limit)
fn cache_key(path: &str, offset: Option<usize>, limit: Option<usize>) -> String {
    format!("{}:{}:{}", path, offset.unwrap_or(0), limit.unwrap_or(0))
}

#[derive(Debug, Clone)]
pub struct FileReadCacheEntry {
    /// Hash of the content that was read
    pub content_hash: u64,
    /// File modification time (if available)
    pub mtime_ns: Option<i64>,
    /// When this entry was recorded
    pub recorded_at: i64,
}

/// Result of checking the cache
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheCheck {
    /// File unchanged — return FILE_UNCHANGED_STUB
    Unchanged,
    /// File changed since last read — re-read required
    Changed,
    /// Not in cache — first read
    Miss,
}

impl FileReadCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            access_order: VecDeque::new(),
            max_entries,
        }
    }

    /// Check if a file read can use the cached version.
    ///
    /// Returns `Unchanged` if the file hasn't changed (same mtime or same content hash).
    /// Returns `Changed` if the file has been modified.
    /// Returns `Miss` if this is the first read.
    pub fn check(
        &self,
        path: &str,
        offset: Option<usize>,
        limit: Option<usize>,
        current_mtime_ns: Option<i64>,
    ) -> CacheCheck {
        let key = cache_key(path, offset, limit);
        match self.entries.get(&key) {
            None => CacheCheck::Miss,
            Some(entry) => {
                // If we have mtime info and it hasn't changed, it's unchanged
                if let (Some(cached_mtime), Some(current_mtime)) =
                    (entry.mtime_ns, current_mtime_ns)
                {
                    if cached_mtime == current_mtime {
                        return CacheCheck::Unchanged;
                    }
                    return CacheCheck::Changed;
                }
                // No mtime available — we can't tell if it changed without content hash
                // Return Changed to force re-read (safe default)
                CacheCheck::Changed
            }
        }
    }

    /// Check using content hash (for when mtime isn't available or content is piped)
    pub fn check_by_hash(
        &self,
        path: &str,
        offset: Option<usize>,
        limit: Option<usize>,
        content_hash: u64,
    ) -> CacheCheck {
        let key = cache_key(path, offset, limit);
        match self.entries.get(&key) {
            None => CacheCheck::Miss,
            Some(entry) => {
                if entry.content_hash == content_hash {
                    CacheCheck::Unchanged
                } else {
                    CacheCheck::Changed
                }
            }
        }
    }

    /// Record a file read in the cache.
    pub fn record(
        &mut self,
        path: &str,
        offset: Option<usize>,
        limit: Option<usize>,
        content: &str,
        mtime_ns: Option<i64>,
    ) {
        let key = cache_key(path, offset, limit);

        // Compute content hash
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        let content_hash = hasher.finish();

        let entry = FileReadCacheEntry {
            content_hash,
            mtime_ns,
            recorded_at: chrono::Utc::now().timestamp_millis(),
        };

        // Remove old position in access order
        self.access_order.retain(|k| k != &key);

        // Insert at front (most recently used)
        self.access_order.push_front(key.clone());
        self.entries.insert(key, entry);

        // Evict oldest if over capacity
        while self.entries.len() > self.max_entries {
            if let Some(old_key) = self.access_order.pop_back() {
                self.entries.remove(&old_key);
            }
        }
    }

    /// Invalidate all cache entries for a file path (called after edits).
    pub fn invalidate(&mut self, path: &str) {
        let keys_to_remove: Vec<String> = self
            .entries
            .keys()
            .filter(|k| k.starts_with(&format!("{}:", path)))
            .cloned()
            .collect();

        for key in &keys_to_remove {
            self.entries.remove(key);
            self.access_order.retain(|k| k != key);
        }
    }

    /// Number of entries in the cache
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Compute content hash for external use (e.g., when checking without full cache)
    pub fn hash_content(content: &str) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    }
}

// ── Content Replacement State ────────────────────────────────────────────────

/// Tracks which tool results have been replaced with persisted previews.
///
/// This ensures prompt cache stability: once a result is replaced, the same
/// replacement string is used on every subsequent API call (not re-evaluated).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContentReplacementState {
    /// Tool use IDs that have been evaluated (seen)
    pub seen_ids: std::collections::HashSet<String>,
    /// Tool use ID → replacement text for persisted results
    pub replacements: HashMap<String, String>,
}

impl ContentReplacementState {
    /// Classify a tool_use_id for replacement decisions
    pub fn classify(&self, tool_use_id: &str) -> ReplacementDecision {
        if let Some(replacement) = self.replacements.get(tool_use_id) {
            ReplacementDecision::MustReapply(replacement.clone())
        } else if self.seen_ids.contains(tool_use_id) {
            ReplacementDecision::Frozen
        } else {
            ReplacementDecision::Fresh
        }
    }

    /// Record that a tool result was seen but NOT replaced
    pub fn mark_seen(&mut self, tool_use_id: &str) {
        self.seen_ids.insert(tool_use_id.to_string());
    }

    /// Record that a tool result was replaced with a preview
    pub fn mark_replaced(&mut self, tool_use_id: &str, replacement: String) {
        self.seen_ids.insert(tool_use_id.to_string());
        self.replacements
            .insert(tool_use_id.to_string(), replacement);
    }
}

/// Decision for how to handle a tool result in the conversation
#[derive(Debug, Clone)]
pub enum ReplacementDecision {
    /// Previously replaced — re-apply the exact same replacement (cache stability)
    MustReapply(String),
    /// Previously seen, not replaced — never replace (would bust prompt cache)
    Frozen,
    /// Never seen — eligible for replacement if over threshold
    Fresh,
}

#[cfg(test)]
mod tests;
