use crate::prompt::{PromptRegistry, TemplateData, build_prompt_messages_with_budget, rough_token_count};
use crate::core::Message;

#[test]
fn rough_token_count_basic() {
    assert_eq!(rough_token_count(""), 0);
    assert_eq!(rough_token_count("abcd"), 1);
    assert_eq!(rough_token_count("Hello world"), 3);
    assert_eq!(rough_token_count("a"), 1);
}

#[tokio::test]
async fn section_cache_hit() {
    let registry = PromptRegistry::with_defaults().await.unwrap();
    let data = TemplateData { instructions: "cached content".into(), ..Default::default() };
    let (c1, t1) = registry.render_section_cached("test", "{{instructions}}", &data).await.unwrap();
    let (c2, t2) = registry.render_section_cached("test", "{{instructions}}", &data).await.unwrap();
    assert_eq!(c1, c2);
    assert_eq!(t1, t2);
}

#[tokio::test]
async fn section_cache_miss_after_invalidation() {
    let registry = PromptRegistry::with_defaults().await.unwrap();
    let data = TemplateData { instructions: "original".into(), ..Default::default() };
    let (c1, _) = registry.render_section_cached("k", "{{instructions}}", &data).await.unwrap();
    assert_eq!(c1, "original");
    registry.invalidate_section("k").await;
    let data2 = TemplateData { instructions: "updated".into(), ..Default::default() };
    let (c2, _) = registry.render_section_cached("k", "{{instructions}}", &data2).await.unwrap();
    assert_eq!(c2, "updated");
}

#[tokio::test]
async fn clear_section_cache_wipes_all() {
    let registry = PromptRegistry::with_defaults().await.unwrap();
    let data = TemplateData { instructions: "x".into(), ..Default::default() };
    registry.render_section_cached("a", "{{instructions}}", &data).await.unwrap();
    registry.clear_section_cache().await;
    assert!(registry.get_static_prefix_hash().await.is_none());
}

#[tokio::test]
async fn static_prefix_hash_deterministic() {
    let registry = PromptRegistry::with_defaults().await.unwrap();
    let data = TemplateData { execution_mode: "tools", tool_format: "json", ..Default::default() };
    let (_, h1, _) = registry.render_static_prefix(&data).await.unwrap();
    registry.clear_section_cache().await;
    let (_, h2, _) = registry.render_static_prefix(&data).await.unwrap();
    assert_eq!(h1, h2);
}

#[tokio::test]
async fn static_prefix_hash_changes_on_format_change() {
    let registry = PromptRegistry::with_defaults().await.unwrap();
    let data1 = TemplateData { execution_mode: "tools", tool_format: "json", ..Default::default() };
    let (_, h1, _) = registry.render_static_prefix(&data1).await.unwrap();
    registry.clear_section_cache().await;
    let data2 = TemplateData { execution_mode: "tools", tool_format: "xml", ..Default::default() };
    let (_, h2, _) = registry.render_static_prefix(&data2).await.unwrap();
    assert_ne!(h1, h2);
}

#[tokio::test]
async fn budget_tracks_tool_tokens_separately() {
    let registry = PromptRegistry::with_defaults().await.unwrap();
    let data = TemplateData {
        instructions: "be helpful".into(),
        available_tools: r#"{"name":"search","params":{"query":"string"}}"#.into(),
        ..Default::default()
    };
    let result = build_prompt_messages_with_budget(
        &registry, "{{instructions}}\n{{{available_tools}}}", "", &data,
        &Message::user("hi".into(), None), 200_000,
    ).await.unwrap();
    assert!(result.budget.tool_schema_tokens > 0);
}

#[tokio::test]
async fn budget_tracks_skill_tokens_separately() {
    let registry = PromptRegistry::with_defaults().await.unwrap();
    let data = TemplateData {
        instructions: "be helpful".into(),
        available_skills: Some("- search: Search the web\n- browser: Browse".into()),
        ..Default::default()
    };
    let result = build_prompt_messages_with_budget(
        &registry, "{{instructions}}", "", &data,
        &Message::user("hi".into(), None), 200_000,
    ).await.unwrap();
    assert!(result.budget.skill_listing_tokens > 0);
}
