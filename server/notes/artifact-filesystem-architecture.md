# Artifact Filesystem Architecture

## Overview

This document describes the new filesystem-based architecture for handling large tool results and context efficiency in Distri agents. The system automatically manages large content by storing it in the filesystem while keeping lightweight metadata references in memory for efficient context management.

## Motivation

Based on context engineering principles from [Manus](https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus) and [LangChain Deep Agents](https://blog.langchain.com/deep-agents/), we identified that:

1. **Context Size Management**: Large tool results consume significant LLM context space
2. **KV-Cache Optimization**: Consistent context prefixes improve cache hit rates  
3. **Filesystem as Ultimate Context**: Use filesystem for unlimited, persistent storage
4. **Metadata Approach**: Keep lightweight references instead of full content

## Architecture Components

### 1. distri-filesystem Crate

**Core Components:**
- `FileSystemStore`: Main storage implementation with namespaced file organization
- `FileSystemOps` trait: Abstract interface for filesystem operations
- `FileMetadata`: Lightweight metadata for stored files
- `store_and_return_metadata()`: Automatic storage function called from AgentExecutor
- `extract_context()`: File type specific context extraction strategies
- Directory walker with glob pattern support for context generation
- Grep-based search tools (grep-regex, grep-searcher)
- JQL JSON query engine integration

**Key Features:**
- Automatic size thresholds (no manual configuration needed)
- Workspace filesystem rooted at `CURRENT_WORKING_DIR` for editable code (`agents/`, `src/`, `plugins/`). This is the same tree exposed through `/v1/files`, so local edits, server writes, and object storage deployments all point at the exact same root.
- Session and artifact data are isolated under `${CURRENT_WORKING_DIR}/.distri/runtime/…`. Every run receives a namespace such as `.distri/runtime/runs/{thread_id}/{task_id}/tool_call_{tool_call_id}.json`, so per-task artifacts never pollute the workspace tree but still share the same object store backend.
- Content type detection with proper file extensions
- JSON metadata with schema detection and statistics
- Handlebars template integration for context generation
- Recursive file tools for understanding context
- `distri-server` always reads and writes via the shared workspace filesystem and never reaches into the runtime prefix directly; artifacts continue to flow through the namespaced `.distri/runtime` prefixes.

### 2. Enhanced ToolResponse Type

**New Structure:**
```rust
pub enum ToolResponseContent {
    Direct { result: serde_json::Value },
    FileReference { 
        metadata: FileMetadata,
        preview: Option<String>,
    },
}
```

**Benefits:**
- Backward compatibility through `result()` method
- Automatic preview generation for context
- File metadata includes size, type, and summary information

### 3. Part Enum Extension

**New Part Type:**
```rust
pub enum Part {
    // ... existing types
    FileMetadata(FileMetadata), // New type for large content references
}
```

### 4. FileSystemArtifactStore

**Hybrid Storage Strategy:**
- Small content (< threshold): Stored directly in memory
- Large content (> threshold): Stored in filesystem with metadata reference
- Automatic detection and transparent handling

## Usage Patterns

### 1. Automatic Tool Result Processing in AgentExecutor

```rust
// In handle_tool_responses - automatic threshold detection
let file_system = self.file_system.clone(); // Arc<FileSystem>
let metadata = store_and_return_metadata(
    &file_system,
    tool_response_data,
    thread_id,
    task_id,
    tool_call_id,
).await?;

// Automatically stores files based on:
// - JSON: Creates metadata with 100 row preview + optional schema
// - Size threshold: Configured defaults (no manual setup)
// - File extension: Based on content type detection
```

### 2. Agent Context Optimization

**Before (Large Content):**
```
System: You are an agent...
User: Analyze this data
Assistant: I'll analyze the data
Tool: search_web -> [50KB of HTML content]
User: What did you find?
```

**After (File References):**
```
System: You are an agent...
User: Analyze this data  
Assistant: I'll analyze the data
Tool: search_web -> FileRef[web_results.html, 50KB, "<!DOCTYPE html><head><title>Search Results..."]
User: What did you find?
```

### 3. File Resolution for Processing

```rust
// When agent needs full content:
if let Some(metadata) = tool_response.get_file_metadata() {
    let full_content = processor.resolve_file_content(metadata).await?;
    // Process full content...
}
```

## Configuration

### FileSystemConfig Options

```rust
FileSystemConfig {
    base_path: current_working_dir().join(".distri/runtime"),
    size_threshold: 50 * 1024, // 50KB
    namespace_strategy: NamespaceStrategy::ThreadId,
}

fn current_working_dir() -> PathBuf {
    std::env::var("CURRENT_WORKING_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
```

**Namespace Strategies:**
- `ThreadId`: `/{thread_id}/{file_id}` (default)
- `TaskId`: `/{task_id}/{file_id}`  
- `Flat`: `/{file_id}`
- `Custom(template)`: Custom template with placeholders

### Size Thresholds

**Recommended Thresholds:**
- Development: 10KB (aggressive caching)
- Production: 50KB (balanced)  
- High-memory: 100KB (conservative)

## Context Engineering Benefits

### 1. Stable Context Prefixes
- Agent instructions remain consistent
- File references don't change context structure
- Improves KV-cache hit rates

### 2. Metadata-Driven Processing
- Agents see file summaries immediately
- Can decide whether to load full content
- Preserves context space for reasoning

### 3. Progressive Context Loading
```
[Initial Context] -> [File Summary] -> [Full Content if needed]
```

### 4. Long-term Memory
- Files persist beyond conversation
- Thread-based organization
- Background cleanup of old files

## Implementation Details

### File Storage Structure
```
${CURRENT_WORKING_DIR}/filesystem/
├── thread_abc123/
│   └── task_def456/
│       ├── tool_call_001.json    # JSON tool result with schema
│       ├── tool_call_002.html    # Web scraping result
│       └── tool_call_003.csv     # Data export
└── thread_ghi789/
    └── task_jkl012/
        └── tool_call_004.json    # Another JSON result
```

The same working directory simultaneously exposes `agents/`, raw files managed by `distrijs/packages/fs`, and `plugins/`. The server exposes CRUD endpoints over this filesystem, mirrors updates into the orchestrator workspace, and expects the frontend to sync via IndexedDB (optimistic edits) → `save` request pipeline (persist to server, then downstream storage).

### Metadata Schema with Type-Specific Statistics
```rust
FileMetadata {
    file_id: "tool_call_001.json",
    relative_path: "thread_abc123/task_def456/tool_call_001.json", 
    size: 51200, // 50KB
    content_type: Some("application/json"),
    preview: Some("{\"results\": [{\"title\": \"First Result\"..."),
    stats: FileStats::Json(JsonStats {
        is_array: true,
        array_length: Some(250),
        schema: Some(JsonSchema {
            properties: HashMap::from([
                ("results", JsonType::Array),
                ("metadata", JsonType::Object),
                ("pagination", JsonType::Object),
            ]),
        }),
        top_level_keys: vec!["results", "metadata", "pagination"],
        nested_depth: 3,
        unique_values_sample: HashMap::from([
            ("results.*.type", vec!["article", "video", "image"]),
            ("results.*.status", vec!["published", "draft"]),
        ]),
        cardinality_estimates: HashMap::from([
            ("results.*.author", 15), // ~15 unique authors
            ("results.*.category", 5), // ~5 categories
        ]),
    }),
    checksum: Some("a1b2c3d4"),
}

// Type-specific stats enum
enum FileStats {
    Json(JsonStats),
    JsonArray(JsonArrayStats),
    Csv(CsvStats),
    Html(HtmlStats),
    Markdown(MarkdownStats),
    Text(TextStats),
    Binary(BinaryStats),
}

struct JsonStats {
    is_array: bool,
    array_length: Option<usize>,
    schema: Option<JsonSchema>,
    top_level_keys: Vec<String>,
    nested_depth: usize,
    unique_values_sample: HashMap<String, Vec<String>>, // field_path -> sample values
    cardinality_estimates: HashMap<String, usize>, // field_path -> estimated unique count
}

struct JsonArrayStats {
    length: usize,
    element_schema: Option<JsonSchema>,
    sample_elements: Vec<serde_json::Value>, // First few elements
    cardinality_per_field: HashMap<String, usize>,
    data_types_distribution: HashMap<String, HashMap<String, usize>>, // field -> type -> count
}

struct CsvStats {
    rows: usize,
    columns: usize,
    headers: Vec<String>,
    data_types: Vec<CsvDataType>, // inferred type per column
    cardinality_per_column: Vec<usize>,
    sample_values: Vec<Vec<String>>, // First few rows
    null_counts: Vec<usize>, // Null/empty count per column
}

struct HtmlStats {
    title: Option<String>,
    meta_description: Option<String>,
    headings: Vec<(String, usize)>, // (text, level)
    link_count: usize,
    image_count: usize,
    script_tags: usize,
    main_content_length: usize,
    language: Option<String>,
}

struct MarkdownStats {
    headings: Vec<(String, usize)>, // (text, level)
    word_count: usize,
    code_blocks: usize,
    links: usize,
    images: usize,
    tables: usize,
    lists: usize,
    front_matter: Option<String>, // YAML/TOML frontmatter type
}

struct TextStats {
    lines: usize,
    words: usize,
    characters: usize,
    encoding: String,
    language: Option<String>,
    structure_hints: TextStructure, // Detected structure patterns
}

enum TextStructure {
    LogFile { log_level_counts: HashMap<String, usize> },
    ConfigFile { format: ConfigFormat },
    CodeFile { language: String, functions: Vec<String> },
    PlainText,
}

struct BinaryStats {
    format: String, // PDF, PNG, etc.
    metadata: HashMap<String, String>, // Format-specific metadata
}
```

### Content Type Detection
- JSON objects/arrays → `application/json`
- HTML content → `text/html`
- CSV data → `text/csv`
- XML content → `text/xml`
- Plain text → `text/plain`

### Preview Generation
- **JSON**: Show structure with key names and truncated values (100 rows for arrays)
- **Text**: First 300 characters with ellipsis
- **HTML**: Extract title and first content snippet
- **Arrays**: Show first few elements with count and schema information

### Context Generation and Search

**Directory Walker with Glob Support:**
- Recursively walks artifact directories
- Supports glob patterns for file matching (*.json, **/*.html)
- Respects ignore patterns and size limits
- Integrates with Handlebars templates for context assembly

**Search Capabilities:**
- **Grep Integration**: Uses grep-regex and grep-searcher for text search
- **JQL Integration**: JSON query engine for structured data search
- **File Type Strategies**: Different search approaches per file type

**Template Integration:**
```handlebars
<file_list>
{{#each files}}
- {{this.path}} ({{this.size}}, {{this.type}})
  {{#if this.preview}}Preview: {{this.preview}}{{/if}}
{{/each}}
</file_list>
```

**extract_context() Function with Stats-Driven Strategies:**
```rust
async fn extract_context(metadata: &FileMetadata) -> Result<String> {
    match &metadata.stats {
        FileStats::Json(stats) => {
            let mut context = format!("JSON file: {} keys", stats.top_level_keys.len());
            if stats.is_array {
                context.push_str(&format!(" (array with {} elements)", stats.array_length.unwrap_or(0)));
            }
            
            // Use cardinality info for smart sampling
            if let Some(high_cardinality_fields) = stats.cardinality_estimates.iter()
                .filter(|(_, count)| **count > 100)
                .map(|(field, _)| field)
                .collect::<Vec<_>>()
                .first() {
                context.push_str(&format!("\nHigh-cardinality field detected: {}", high_cardinality_fields));
                // Use JQL to extract unique values sample
            }
            
            Ok(context)
        },
        
        FileStats::Csv(stats) => {
            let context = format!(
                "CSV: {} rows × {} columns\nHeaders: {}\nData types: {:?}",
                stats.rows, stats.columns, 
                stats.headers.join(", "),
                stats.data_types
            );
            
            // Include cardinality insights
            let high_card_cols: Vec<_> = stats.cardinality_per_column.iter()
                .enumerate()
                .filter(|(_, &card)| card > stats.rows / 2) // High cardinality columns
                .map(|(i, _)| &stats.headers[i])
                .collect();
                
            if !high_card_cols.is_empty() {
                context.push_str(&format!("\nHigh-cardinality columns: {}", high_card_cols.join(", ")));
            }
            
            Ok(context)
        },
        
        FileStats::Html(stats) => {
            let mut context = format!("HTML document");
            if let Some(title) = &stats.title {
                context.push_str(&format!(": {}", title));
            }
            context.push_str(&format!("\n{} links, {} images, {} headings", 
                stats.link_count, stats.image_count, stats.headings.len()));
            
            // Include heading structure for navigation
            if !stats.headings.is_empty() {
                context.push_str("\nHeadings:");
                for (heading, level) in &stats.headings[..5.min(stats.headings.len())] {
                    context.push_str(&format!("\n  {}: {}", "H".repeat(*level), heading));
                }
            }
            
            Ok(context)
        },
        
        FileStats::Markdown(stats) => {
            let context = format!(
                "Markdown: {} words, {} headings, {} code blocks, {} tables",
                stats.word_count, stats.headings.len(), stats.code_blocks, stats.tables
            );
            Ok(context)
        },
        
        FileStats::Text(stats) => {
            let mut context = format!("Text file: {} lines, {} words", stats.lines, stats.words);
            match &stats.structure_hints {
                TextStructure::LogFile { log_level_counts } => {
                    context.push_str(&format!("\nLog levels: {:?}", log_level_counts));
                },
                TextStructure::CodeFile { language, functions } => {
                    context.push_str(&format!("\n{} code: {} functions", language, functions.len()));
                },
                TextStructure::ConfigFile { format } => {
                    context.push_str(&format!("\nConfig file: {:?}", format));
                },
                TextStructure::PlainText => {},
            }
            Ok(context)
        },
        
        FileStats::Binary(stats) => {
            Ok(format!("Binary file: {} format\nMetadata: {:?}", stats.format, stats.metadata))
        },
    }
}
```

## Error Handling

### Graceful Degradation
- Filesystem errors don't break agent execution
- Falls back to in-memory storage if filesystem unavailable
- Warns but continues if file cleanup fails

### Integrity Verification
- Checksum validation on file reads
- Warns about corrupted files
- Option to regenerate content from original source

### Recovery Strategies
- Missing files marked as unavailable in metadata
- Agent prompts include file recovery instructions
- Background tasks can regenerate lost content

## Performance Considerations

### Memory Usage
- **Before**: 1MB tool result = 1MB memory usage
- **After**: 1MB tool result = ~1KB metadata + disk storage

### Context Efficiency  
- **Before**: 1MB result consumes ~2000 tokens
- **After**: File reference consumes ~50 tokens + preview

### I/O Optimization
- Lazy loading of file content
- Streaming for very large files
- Async operations throughout

## Migration Strategy

### Phase 1: Backward Compatibility
- New `ToolResponseContent` enum with legacy `result()` method
- Existing code continues to work unchanged
- Gradual adoption of file references

### Phase 2: Automatic Detection
- `ToolResponseProcessor` transparently handles large results
- Agents automatically benefit without code changes
- Configurable size thresholds per agent

### Phase 3: Full Integration
- Agent prompts optimized for file metadata
- Context engineering patterns implemented
- Legacy direct result methods deprecated

## Monitoring and Observability

### Metrics to Track
- Filesystem storage usage
- Cache hit rates for file content
- Context size reduction percentages
- Agent performance improvements

### Logging
- File storage operations (with sizes)
- Context size before/after optimization
- File access patterns
- Cleanup operations

## Future Enhancements

### Advanced Features
- **Compression**: Automatic compression for text-based content
- **Deduplication**: Share identical files across threads
- **Encryption**: At-rest encryption for sensitive content
- **Cloud Storage**: S3/GCS backends for distributed deployments

### Agent Integration
- **Smart Loading**: Agents learn which files to load fully
- **Content Indexing**: Search across stored file content
- **Version Control**: Track changes to file content over time

### Performance Optimizations
- **Content Streaming**: Stream large files during processing
- **Background Processing**: Pre-process files for common operations
- **Cache Hierarchy**: Multi-level caching strategy

## Testing Strategy

### Unit Tests
- FileSystemStore operations
- ToolResponse serialization/deserialization  
- Content type detection
- Preview generation

### Integration Tests
- End-to-end tool result processing
- Agent context optimization
- File cleanup operations
- Error recovery scenarios

### Performance Tests
- Large file handling benchmarks
- Memory usage comparison
- Context size reduction measurement
- Cache hit rate optimization

This architecture provides a foundation for efficient context management while maintaining backward compatibility and enabling advanced agent behaviors through filesystem-based storage.
