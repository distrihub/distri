use super::*;

// ── ContentFormat Detection ──────────────────────────────────────────────────

#[test]
fn detect_markdown_by_extension() {
    assert_eq!(
        ContentFormat::detect("some text", Some("readme.md")),
        ContentFormat::Markdown
    );
    assert_eq!(
        ContentFormat::detect("some text", Some("doc.mdx")),
        ContentFormat::Markdown
    );
}

#[test]
fn detect_json_by_extension() {
    assert_eq!(
        ContentFormat::detect("not json", Some("data.json")),
        ContentFormat::Json
    );
}

#[test]
fn detect_json_by_content() {
    let json = r#"{"key": "value", "nested": {"a": 1}}"#;
    assert_eq!(ContentFormat::detect(json, None), ContentFormat::Json);
}

#[test]
fn detect_json_array_by_content() {
    let json = r#"[{"id": 1}, {"id": 2}]"#;
    assert_eq!(ContentFormat::detect(json, None), ContentFormat::Json);
}

#[test]
fn detect_markdown_by_content_heading() {
    let md = "# Title\n\nSome content\n\n## Section\n\nMore content";
    assert_eq!(ContentFormat::detect(md, None), ContentFormat::Markdown);
}

#[test]
fn detect_markdown_by_frontmatter() {
    let md = "---\ntitle: Hello\n---\n\n# Content";
    assert_eq!(ContentFormat::detect(md, None), ContentFormat::Markdown);
}

#[test]
fn detect_csv_by_content() {
    let csv = "name,age,city\nAlice,30,NYC\nBob,25,LA\n";
    assert_eq!(ContentFormat::detect(csv, None), ContentFormat::Csv);
}

#[test]
fn detect_binary_by_extension() {
    assert_eq!(
        ContentFormat::detect("anything", Some("image.png")),
        ContentFormat::Binary
    );
    assert_eq!(
        ContentFormat::detect("anything", Some("archive.zip")),
        ContentFormat::Binary
    );
}

#[test]
fn detect_binary_by_null_bytes() {
    let content = "hello\0world\0binary";
    assert_eq!(ContentFormat::detect(content, None), ContentFormat::Binary);
}

#[test]
fn detect_plain_text_fallback() {
    let text = "Just some regular text content.\nWith multiple lines.\nNothing special.";
    assert_eq!(ContentFormat::detect(text, None), ContentFormat::PlainText);
}

// ── Preview Generation ───────────────────────────────────────────────────────

#[test]
fn preview_small_content_returns_full() {
    let content = "Hello, world!";
    let preview = Preview::generate(content, 2000);
    assert_eq!(preview.text, content);
    assert!(!preview.has_more);
    assert_eq!(preview.total_bytes, content.len());
}

#[test]
fn preview_large_text_truncates_at_boundary() {
    let mut content = String::new();
    for i in 0..100 {
        content.push_str(&format!("Line {} with some content that fills it up\n", i));
    }
    let preview = Preview::generate(&content, 200);
    assert!(preview.has_more);
    assert!(preview.text.len() <= 250); // Some overshoot allowed for boundary
    // Should end at a newline boundary
    assert!(
        preview.text.ends_with('\n'),
        "Preview should end at newline boundary: {:?}",
        &preview.text[preview.text.len().saturating_sub(20)..]
    );
}

#[test]
fn preview_markdown_extracts_headings() {
    let md = format!(
        "# Main Title\n\nIntro paragraph.\n\n## Section 1\n\nContent 1.\n\n## Section 2\n\nContent 2.\n\n## Section 3\n\n{}\n",
        "Long content. ".repeat(500)
    );
    let preview = Preview::generate_for_file(&md, 500, "doc.md");
    assert!(preview.has_more);
    assert_eq!(preview.format, ContentFormat::Markdown);
    assert!(preview.text.contains("# Main Title"));
    assert!(preview.text.contains("## Section 1"));
}

#[test]
fn preview_json_shows_structure() {
    let json = serde_json::json!({
        "users": [
            {"id": 1, "name": "Alice", "bio": "A".repeat(1000)},
            {"id": 2, "name": "Bob", "bio": "B".repeat(1000)},
            {"id": 3, "name": "Charlie", "bio": "C".repeat(1000)},
        ],
        "total": 3,
        "page": 1
    });
    let content = serde_json::to_string_pretty(&json).unwrap();
    let preview = Preview::generate_for_file(&content, 500, "data.json");
    assert!(preview.has_more);
    assert_eq!(preview.format, ContentFormat::Json);
    assert!(preview.text.contains("users"));
    assert!(preview.text.contains("total"));
}

#[test]
fn preview_csv_shows_headers_and_rows() {
    let mut csv = String::from("name,age,city,country\n");
    for i in 0..100 {
        csv.push_str(&format!("Person{},{},City{},Country{}\n", i, 20 + i, i, i));
    }
    let preview = Preview::generate_for_file(&csv, 500, "data.csv");
    assert!(preview.has_more);
    assert_eq!(preview.format, ContentFormat::Csv);
    assert!(preview.text.contains("[Headers]"));
    assert!(preview.text.contains("[Row 1]"));
    assert!(preview.text.contains("more rows"));
}

#[test]
fn preview_binary_shows_notice() {
    let preview = Preview::generate_for_file("binary\0content", 2000, "image.png");
    assert!(preview.text.contains("Binary content"));
    assert!(preview.text.contains("cannot display"));
}

#[test]
fn preview_persisted_notice_format() {
    let content = "x".repeat(10_000);
    let preview = Preview::generate(&content, 2000);
    let notice = preview.as_persisted_notice("/tmp/tool-results/abc123.txt");
    assert!(notice.contains("<persisted-output>"));
    assert!(notice.contains("/tmp/tool-results/abc123.txt"));
    assert!(notice.contains("</persisted-output>"));
}

// ── Persistence Threshold ────────────────────────────────────────────────────

#[test]
fn should_persist_small_content() {
    assert!(!should_persist("small"));
    assert!(!should_persist(&"x".repeat(49_999)));
}

#[test]
fn should_persist_large_content() {
    assert!(should_persist(&"x".repeat(50_000)));
    assert!(should_persist(&"x".repeat(100_000)));
}

// ── File Read Cache ──────────────────────────────────────────────────────────

#[test]
fn cache_miss_on_first_read() {
    let cache = FileReadCache::new(100);
    assert_eq!(
        cache.check("/foo.rs", None, None, Some(1000)),
        CacheCheck::Miss
    );
}

#[test]
fn cache_unchanged_on_same_mtime() {
    let mut cache = FileReadCache::new(100);
    cache.record("/foo.rs", None, None, "content", Some(1000));
    assert_eq!(
        cache.check("/foo.rs", None, None, Some(1000)),
        CacheCheck::Unchanged
    );
}

#[test]
fn cache_changed_on_different_mtime() {
    let mut cache = FileReadCache::new(100);
    cache.record("/foo.rs", None, None, "content", Some(1000));
    assert_eq!(
        cache.check("/foo.rs", None, None, Some(2000)),
        CacheCheck::Changed
    );
}

#[test]
fn cache_unchanged_by_hash() {
    let mut cache = FileReadCache::new(100);
    let content = "hello world";
    cache.record("/foo.rs", None, None, content, None);
    let hash = FileReadCache::hash_content(content);
    assert_eq!(
        cache.check_by_hash("/foo.rs", None, None, hash),
        CacheCheck::Unchanged
    );
}

#[test]
fn cache_changed_by_hash() {
    let mut cache = FileReadCache::new(100);
    cache.record("/foo.rs", None, None, "original", None);
    let new_hash = FileReadCache::hash_content("modified");
    assert_eq!(
        cache.check_by_hash("/foo.rs", None, None, new_hash),
        CacheCheck::Changed
    );
}

#[test]
fn cache_different_offsets_are_separate() {
    let mut cache = FileReadCache::new(100);
    cache.record("/foo.rs", Some(0), Some(100), "first 100", Some(1000));
    cache.record("/foo.rs", Some(100), Some(100), "next 100", Some(1000));

    assert_eq!(
        cache.check("/foo.rs", Some(0), Some(100), Some(1000)),
        CacheCheck::Unchanged
    );
    assert_eq!(
        cache.check("/foo.rs", Some(100), Some(100), Some(1000)),
        CacheCheck::Unchanged
    );
    assert_eq!(
        cache.check("/foo.rs", Some(200), Some(100), Some(1000)),
        CacheCheck::Miss
    );
}

#[test]
fn cache_invalidate_removes_all_offsets() {
    let mut cache = FileReadCache::new(100);
    cache.record("/foo.rs", None, None, "content1", Some(1000));
    cache.record("/foo.rs", Some(0), Some(50), "content2", Some(1000));
    assert_eq!(cache.len(), 2);

    cache.invalidate("/foo.rs");
    assert_eq!(cache.len(), 0);
    assert_eq!(
        cache.check("/foo.rs", None, None, Some(1000)),
        CacheCheck::Miss
    );
}

#[test]
fn cache_lru_eviction() {
    let mut cache = FileReadCache::new(3);
    cache.record("/a.rs", None, None, "a", Some(1));
    cache.record("/b.rs", None, None, "b", Some(2));
    cache.record("/c.rs", None, None, "c", Some(3));
    assert_eq!(cache.len(), 3);

    cache.record("/d.rs", None, None, "d", Some(4));
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.check("/a.rs", None, None, Some(1)), CacheCheck::Miss);
    assert_eq!(
        cache.check("/d.rs", None, None, Some(4)),
        CacheCheck::Unchanged
    );
}

#[test]
fn cache_lru_access_refreshes_order() {
    let mut cache = FileReadCache::new(3);
    cache.record("/a.rs", None, None, "a", Some(1));
    cache.record("/b.rs", None, None, "b", Some(2));
    cache.record("/c.rs", None, None, "c", Some(3));

    // Re-record /a.rs (refresh)
    cache.record("/a.rs", None, None, "a", Some(1));

    // /d.rs evicts /b.rs (oldest after refresh)
    cache.record("/d.rs", None, None, "d", Some(4));
    assert_eq!(
        cache.check("/a.rs", None, None, Some(1)),
        CacheCheck::Unchanged
    );
    assert_eq!(cache.check("/b.rs", None, None, Some(2)), CacheCheck::Miss);
}

// ── Content Replacement State ────────────────────────────────────────────────

#[test]
fn replacement_state_fresh_then_frozen() {
    let mut state = ContentReplacementState::default();
    assert!(matches!(state.classify("tc1"), ReplacementDecision::Fresh));

    state.mark_seen("tc1");
    assert!(matches!(state.classify("tc1"), ReplacementDecision::Frozen));
}

#[test]
fn replacement_state_replaced_reapplies() {
    let mut state = ContentReplacementState::default();
    state.mark_replaced("tc1", "preview text here".to_string());

    match state.classify("tc1") {
        ReplacementDecision::MustReapply(text) => assert_eq!(text, "preview text here"),
        other => panic!("Expected MustReapply, got {:?}", other),
    }
}

// ── Integration Scenarios ────────────────────────────────────────────────────

#[test]
fn scenario_50k_file_read_persistence() {
    let content = "fn main() {\n".to_string()
        + &(0..2000)
            .map(|i| format!("    println!(\"line {}\");\n", i))
            .collect::<String>()
        + "}\n";
    assert!(content.len() > 50_000);
    assert!(should_persist(&content));

    let preview = Preview::generate_for_file(&content, PREVIEW_SIZE_BYTES, "main.rs");
    assert!(preview.has_more);
    assert!(preview.text.len() <= PREVIEW_SIZE_BYTES + 100);
    assert_eq!(preview.format, ContentFormat::PlainText);
    assert!(preview.text.ends_with('\n'));

    let notice = preview.as_persisted_notice("tool-results/tc1.txt");
    assert!(notice.contains("<persisted-output>"));
}

#[test]
fn scenario_unchanged_file_dedup() {
    let mut cache = FileReadCache::new(100);
    let content = "fn foo() { 42 }";
    let mtime = Some(1000i64);

    assert_eq!(
        cache.check("/src/lib.rs", None, None, mtime),
        CacheCheck::Miss
    );
    cache.record("/src/lib.rs", None, None, content, mtime);

    assert_eq!(
        cache.check("/src/lib.rs", None, None, mtime),
        CacheCheck::Unchanged
    );
    assert_eq!(
        cache.check("/src/lib.rs", None, None, Some(2000)),
        CacheCheck::Changed
    );
}

#[test]
fn scenario_large_json_api_response() {
    let data = serde_json::json!({
        "results": (0..100).map(|i| serde_json::json!({
            "id": i,
            "name": format!("Item {}", i),
            "description": "A".repeat(500),
        })).collect::<Vec<_>>(),
        "pagination": {"page": 1, "total_pages": 10, "total_items": 1000}
    });
    let content = serde_json::to_string_pretty(&data).unwrap();
    assert!(content.len() > 50_000);

    let preview = Preview::generate_for_file(&content, PREVIEW_SIZE_BYTES, "response.json");
    assert!(preview.has_more);
    assert_eq!(preview.format, ContentFormat::Json);
    assert!(preview.text.contains("results"));
    assert!(preview.text.contains("pagination"));
}

#[test]
fn scenario_large_markdown_doc() {
    let mut md = String::from("# API Reference\n\nComplete API documentation.\n\n");
    for i in 0..50 {
        md.push_str(&format!("## Endpoint {}\n\n", i));
        md.push_str(&"Parameters and detailed usage info. ".repeat(20));
        md.push('\n');
    }
    assert!(md.len() > 20_000);

    let preview = Preview::generate_for_file(&md, PREVIEW_SIZE_BYTES, "api.md");
    assert!(preview.has_more);
    assert_eq!(preview.format, ContentFormat::Markdown);
    assert!(preview.text.contains("# API Reference"));
}

#[test]
fn scenario_binary_file_rejection() {
    let binary = "ELF\0\x01\x01\x01\0\0\0\0\0\0\0\0\0";
    assert!(is_binary_content(binary, Some("program.exe")));
    assert!(is_binary_content(binary, None));

    let preview = Preview::generate_for_file(binary, 2000, "program.exe");
    assert_eq!(preview.format, ContentFormat::Binary);
    assert!(preview.text.contains("Binary content"));
}

#[test]
fn scenario_csv_large_dataset() {
    let mut csv = String::from("id,name,email,department,salary\n");
    for i in 0..10_000 {
        csv.push_str(&format!(
            "{},Employee{},emp{}@co.com,Dept{},{}\n",
            i,
            i,
            i,
            i % 10,
            50000 + i * 100
        ));
    }
    assert!(csv.len() > 100_000);

    let preview = Preview::generate_for_file(&csv, PREVIEW_SIZE_BYTES, "employees.csv");
    assert!(preview.has_more);
    assert_eq!(preview.format, ContentFormat::Csv);
    assert!(preview.text.contains("[Headers]"));
    assert!(preview.text.contains("id,name,email"));
    assert!(preview.text.contains("more rows"));
}

#[test]
fn scenario_replacement_state_cache_stability() {
    let mut state = ContentReplacementState::default();

    // Turn 1: tc1 is large, gets replaced. tc2 is small, stays.
    state.mark_replaced("tc1", "<persisted>preview of tc1</persisted>".to_string());
    state.mark_seen("tc2");

    // Turn 2: Both are now locked in
    assert!(matches!(
        state.classify("tc1"),
        ReplacementDecision::MustReapply(_)
    ));
    assert!(matches!(state.classify("tc2"), ReplacementDecision::Frozen));

    // Turn 2: tc3 is new and eligible for replacement
    assert!(matches!(state.classify("tc3"), ReplacementDecision::Fresh));
}
