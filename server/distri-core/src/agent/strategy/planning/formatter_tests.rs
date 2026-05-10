    use super::*;
    use crate::types::{AgentStrategy, Message};
    use distri_types::{
        Action, ExecutionHistoryEntry, ExecutionStatus, ModelProvider, ModelSettings, PlanStep,
        ScratchpadEntry, ScratchpadEntryType, ToolCall, ToolResponse,
    };
    use serde_json::json;

    fn base_agent_definition(
        provider: ModelProvider,
        format: ToolCallFormat,
    ) -> crate::types::StandardDefinition {
        crate::types::StandardDefinition {
            name: "test".to_string(),
            instructions: "Be helpful".to_string(),
            model_settings: Some(ModelSettings {
                model: "test-model".to_string(),
                inner: distri_types::ModelSettingsInner {
                    provider,
                    ..Default::default()
                },
            }),
            tool_format: format,
            ..Default::default()
        }
    }

    fn sample_execution_result() -> ExecutionResult {
        let tool_call = ToolCall {
            tool_call_id: "call-1".to_string(),
            tool_name: "apply_ops".to_string(),
            input: json!({"ops": []}),
        };
        let tool_response = ToolResponse::direct(
            "call-1".to_string(),
            "apply_ops".to_string(),
            json!({"result": {"success": true}}),
        );

        ExecutionResult {
            step_id: "step-1".to_string(),
            parts: vec![Part::ToolCall(tool_call), Part::ToolResult(tool_response)],
            status: ExecutionStatus::Success,
            reason: None,
            timestamp: 1,
        }
    }

    fn sample_plan_entry() -> ScratchpadEntry {
        ScratchpadEntry {
            timestamp: 0,
            entry_type: ScratchpadEntryType::PlanStep(PlanStep {
                id: "step-1".to_string(),
                thought: Some("Think".to_string()),
                action: Action::ToolCalls {
                    tool_calls: vec![ToolCall {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "search".to_string(),
                        input: json!({"query": "rust"}),
                    }],
                },
            }),
            task_id: "task".to_string(),
            parent_task_id: None,
            entry_kind: Some("task".to_string()),
        }
    }

    fn sample_execution_entry() -> ScratchpadEntry {
        ScratchpadEntry {
            timestamp: 1,
            entry_type: ScratchpadEntryType::Execution(ExecutionHistoryEntry {
                thread_id: "thread".to_string(),
                task_id: "task".to_string(),
                run_id: "run".to_string(),
                execution_result: sample_execution_result(),
                stored_at: 1,
            }),
            task_id: "task".to_string(),
            parent_task_id: None,
            entry_kind: Some("task".to_string()),
        }
    }

    fn sample_large_execution_entry(step_id: &str, timestamp: i64) -> ScratchpadEntry {
        ScratchpadEntry {
            timestamp,
            entry_type: ScratchpadEntryType::Execution(ExecutionHistoryEntry {
                thread_id: "thread".to_string(),
                task_id: "task".to_string(),
                run_id: "run".to_string(),
                execution_result: ExecutionResult {
                    step_id: step_id.to_string(),
                    parts: vec![Part::Data(json!({"huge": "x".repeat(5000)}))],
                    status: ExecutionStatus::Success,
                    reason: None,
                    timestamp,
                },
                stored_at: timestamp,
            }),
            task_id: "task".to_string(),
            parent_task_id: None,
            entry_kind: Some("task".to_string()),
        }
    }

    #[test]
    fn interleave_user_and_tool_history_groups_tools_between_users() {
        let mut u1 = Message::user("u1".to_string(), None);
        u1.created_at = 10;
        let mut u2 = Message::user("u2".to_string(), None);
        u2.created_at = 40;

        let mut assistant = Message::assistant("assistant".to_string(), None);
        assistant.created_at = 20;

        let mut tool = Message::tool_response(
            "call".to_string(),
            "search".to_string(),
            &json!({"result": true}),
        );
        tool.created_at = 30;
        tool.role = MessageRole::Tool;

        let mut current = Message::user("current".to_string(), None);
        current.created_at = 40;
        current.id = u2.id.clone();

        let interleaved = MessageFormatter::interleave_user_and_tool_history(
            vec![u2, u1],
            vec![assistant, tool],
            &current,
        );

        let order: Vec<_> = interleaved
            .iter()
            .map(|message| (message.role.clone(), message.created_at))
            .collect();
        assert_eq!(
            order,
            vec![
                (MessageRole::User, 10),
                (MessageRole::Assistant, 20),
                (MessageRole::Tool, 30),
                (MessageRole::User, 40)
            ]
        );
    }

    #[test]
    fn interleave_user_and_tool_history_replaces_current_user_message() {
        let mut stored_current = Message::user("stored".to_string(), None);
        stored_current.created_at = 10;

        let mut tool = Message::tool_response(
            "call".to_string(),
            "search".to_string(),
            &json!({"result": true}),
        );
        tool.created_at = 20;
        tool.role = MessageRole::Tool;

        let mut current = Message::user("enriched".to_string(), None);
        current.created_at = 10;
        current.id = stored_current.id.clone();

        let interleaved = MessageFormatter::interleave_user_and_tool_history(
            vec![stored_current],
            vec![tool],
            &current,
        );

        assert_eq!(interleaved[0].role, MessageRole::User);
        assert_eq!(interleaved[0].as_text().unwrap(), "enriched");
    }

    #[tokio::test]
    async fn native_history_uses_scratchpad_entries() {
        let native = MessageFormatter::build_native_history_messages(&[
            sample_plan_entry(),
            sample_execution_entry(),
        ]);

        assert_eq!(native.len(), 2);
        assert!(matches!(native[0].role, MessageRole::Assistant));
        assert!(matches!(native[1].role, MessageRole::Tool));
        assert_eq!(native[1].tool_responses().len(), 1);
        assert_eq!(native[0].tool_calls().len(), 1);
    }

    #[tokio::test]
    async fn fallback_history_from_execution_results() {
        let messages = MessageFormatter::build_native_history_messages(&[]);
        assert!(messages.is_empty());
    }

    fn sample_image_tool_result_entry(
        step_id: &str,
        timestamp: i64,
        image_marker: &str,
    ) -> ScratchpadEntry {
        use distri_types::{FileType, ToolResponse};
        let tool_result = ToolResponse {
            tool_call_id: format!("tc-{}", step_id),
            tool_name: "db_get".to_string(),
            parts: vec![
                Part::Data(json!({"id": step_id})),
                Part::Image(FileType::Bytes {
                    bytes: image_marker.to_string(),
                    mime_type: "image/jpeg".to_string(),
                    name: None,
                }),
            ],
            parts_metadata: None,
        };
        ScratchpadEntry {
            timestamp,
            entry_type: ScratchpadEntryType::Execution(ExecutionHistoryEntry {
                thread_id: "thread".to_string(),
                task_id: "task".to_string(),
                run_id: "run".to_string(),
                execution_result: ExecutionResult {
                    step_id: step_id.to_string(),
                    parts: vec![Part::ToolResult(tool_result)],
                    status: ExecutionStatus::Success,
                    reason: None,
                    timestamp,
                },
                stored_at: timestamp,
            }),
            task_id: "task".to_string(),
            parent_task_id: None,
            entry_kind: Some("execution".to_string()),
        }
    }

    /// Pins the §3 invariant in `docs/execution/scratchpad.md`: the LATEST
    /// execution entry's tool result keeps its inline image; older entries
    /// have it replaced with the placeholder text.
    ///
    /// If this regresses, a worker that just `db_get`-ed an image (e.g.
    /// the importer) will be unable to OCR it on the next planning turn —
    /// the LLM client only sees a placeholder string instead of a real
    /// `image_url` content part.
    #[test]
    fn latest_execution_entry_preserves_inline_image_older_strips_it() {
        use distri_types::FileType;
        let entries = vec![
            sample_image_tool_result_entry("step-old", 1, "OLD-IMAGE-BYTES"),
            sample_image_tool_result_entry("step-latest", 2, "LATEST-IMAGE-BYTES"),
        ];

        let messages = MessageFormatter::build_native_history_messages(&entries);
        // Two execution entries → each renders as one assistant + one tool
        // message via execution_result_to_messages, giving 4 messages total.
        assert!(
            messages.len() >= 2,
            "expected at least two messages from two execution entries; got {}",
            messages.len()
        );

        // Find the tool messages; each carries a single ToolResponse.
        let tool_msgs: Vec<_> = messages
            .iter()
            .filter(|m| matches!(m.role, MessageRole::Tool))
            .collect();
        assert_eq!(tool_msgs.len(), 2, "expected one tool message per entry");

        // The OLDER tool message: image should be stripped to placeholder.
        let older_inner_parts = match &tool_msgs[0].parts[0] {
            Part::ToolResult(tr) => &tr.parts,
            other => panic!("expected ToolResult; got {:?}", other),
        };
        let old_has_placeholder = older_inner_parts.iter().any(|p| match p {
            Part::Text(t) => t.contains("Image omitted"),
            _ => false,
        });
        assert!(
            old_has_placeholder,
            "older entry's image should be stripped at history; parts = {:?}",
            older_inner_parts
        );

        // The LATEST tool message: image should be present verbatim.
        let latest_inner_parts = match &tool_msgs[1].parts[0] {
            Part::ToolResult(tr) => &tr.parts,
            other => panic!("expected ToolResult; got {:?}", other),
        };
        let latest_image_bytes = latest_inner_parts.iter().find_map(|p| match p {
            Part::Image(FileType::Bytes { bytes, .. }) => Some(bytes.clone()),
            _ => None,
        });
        assert_eq!(
            latest_image_bytes.as_deref(),
            Some("LATEST-IMAGE-BYTES"),
            "latest entry's image must be intact for the next LLM turn; parts = {:?}",
            latest_inner_parts
        );
    }

    /// With a single execution entry there's nothing older to strip; the
    /// image must be preserved.
    #[test]
    fn single_execution_entry_keeps_inline_image() {
        use distri_types::FileType;
        let entries = vec![sample_image_tool_result_entry("only", 1, "ONLY-BYTES")];
        let messages = MessageFormatter::build_native_history_messages(&entries);
        let tool_msg = messages
            .iter()
            .find(|m| matches!(m.role, MessageRole::Tool))
            .expect("expected one tool message");
        let inner = match &tool_msg.parts[0] {
            Part::ToolResult(tr) => &tr.parts,
            _ => panic!("expected ToolResult"),
        };
        let has_image = inner.iter().any(|p| match p {
            Part::Image(FileType::Bytes { bytes, .. }) => bytes == "ONLY-BYTES",
            _ => false,
        });
        assert!(
            has_image,
            "single (latest) entry must retain its inline image"
        );
    }

    #[test]
    fn native_history_compacts_only_until_n_minus_1() {
        let entries = vec![
            sample_large_execution_entry("step-1", 1),
            sample_large_execution_entry("step-2", 2),
        ];

        let messages = MessageFormatter::build_native_history_messages(&entries);
        assert_eq!(messages.len(), 2);

        let first_data = messages[0]
            .parts
            .iter()
            .find_map(|part| match part {
                Part::Data(value) => Some(value.clone()),
                _ => None,
            })
            .expect("first message should contain data part");
        assert_eq!(first_data["truncated"], json!(true));

        let second_data = messages[1]
            .parts
            .iter()
            .find_map(|part| match part {
                Part::Data(value) => Some(value.clone()),
                _ => None,
            })
            .expect("second message should contain data part");
        assert!(second_data.get("truncated").is_none());
    }

    #[tokio::test]
    async fn openai_messages_include_tool_history_when_native() {
        let agent_def = base_agent_definition(ModelProvider::OpenAI {}, ToolCallFormat::Provider);
        let strategy = AgentStrategy::default();
        let formatter = MessageFormatter::new(&agent_def, &strategy);
        let context = Arc::new(ExecutorContext::default());
        let user_msg = Message::user("Plan".to_string(), None);

        // No orchestrator in this unit test context, so no execution history is available.
        let (messages, _) = formatter
            .build_messages(&user_msg, &context, "tmpl", "user_templ", None)
            .await
            .expect("formatter should succeed");

        assert_eq!(messages.len(), 2);
        assert!(matches!(messages[0].role, MessageRole::System));
        assert!(matches!(messages[1].role, MessageRole::User));
        let user_text = messages[1].as_text().unwrap_or_default();
        assert!(user_text.contains("user_templ"));
    }

    #[tokio::test]
    async fn non_openai_prefers_system_and_user_only() {
        let agent_def = base_agent_definition(ModelProvider::OpenAI {}, ToolCallFormat::JsonL);
        let strategy = AgentStrategy::default();
        let formatter = MessageFormatter::new(&agent_def, &strategy);
        let context = Arc::new(ExecutorContext::default());
        let user_msg = Message::user("Summarize context".to_string(), None);
        // No orchestrator in this unit test context, so no execution history is available.
        let (messages, _) = formatter
            .build_messages(
                &user_msg,
                &context,
                "tmpl",
                "user_templ",
                Some("Todo".to_string()),
            )
            .await
            .expect("formatter should succeed");

        assert_eq!(messages.len(), 2);
        assert!(matches!(messages[0].role, MessageRole::System));
        assert!(matches!(messages[1].role, MessageRole::User));
        let user_text = messages[1].as_text().unwrap_or_default();
        assert!(user_text.contains("user_templ"));
    }

    #[test]
    fn collect_tool_prompts_skips_deferred() {
        let defs = vec![
            distri_types::ToolDefinition {
                name: "Bash".into(),
                description: "Run shell".into(),
                parameters: json!({}),
                prompt: Some("Use Bash for shell commands.".into()),
                examples: None,
                output_schema: None,
            },
            distri_types::ToolDefinition {
                name: "browsr_scrape".into(),
                description: "Scrape web".into(),
                parameters: json!({}),
                prompt: Some("Scrape websites for data.".into()),
                examples: None,
                output_schema: None,
            },
            distri_types::ToolDefinition {
                name: "final".into(),
                description: "Final answer".into(),
                parameters: json!({}),
                prompt: None,
                examples: None,
                output_schema: None,
            },
        ];

        let deferred: std::collections::HashSet<String> =
            ["browsr_scrape".to_string()].into_iter().collect();
        let (map, concat, entries) = MessageFormatter::collect_tool_prompts(&defs, &deferred);

        // All tools present in map (deferred gets empty string)
        assert_eq!(map.len(), 3);
        assert_eq!(
            map.get("Bash").unwrap().as_str().unwrap(),
            "Use Bash for shell commands."
        );
        assert_eq!(map.get("browsr_scrape").unwrap().as_str().unwrap(), ""); // deferred = empty
        assert_eq!(map.get("final").unwrap().as_str().unwrap(), "");

        // Concatenated prompts should NOT include deferred tool's prompt
        assert!(concat.contains("Bash"));
        assert!(!concat.contains("Scrape websites"));

        // Entries should only have non-deferred tools with prompts
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Bash");
    }

    #[test]
    fn tool_prompts_render_in_handlebars_template() {
        let defs = vec![
            distri_types::ToolDefinition {
                name: "Glob".into(),
                description: "Find files".into(),
                parameters: json!({}),
                prompt: Some("Use Glob for file patterns.".into()),
                examples: None,
                output_schema: None,
            },
            distri_types::ToolDefinition {
                name: "Grep".into(),
                description: "Search".into(),
                parameters: json!({}),
                prompt: Some("Use Grep for content search.".into()),
                examples: None,
                output_schema: None,
            },
        ];

        let no_deferred = std::collections::HashSet::new();
        let (map, _concat, _entries) = MessageFormatter::collect_tool_prompts(&defs, &no_deferred);

        let mut dynamic_values = std::collections::HashMap::new();
        dynamic_values.insert("tools".to_string(), serde_json::Value::Object(map));

        let mut hbs = handlebars::Handlebars::new();
        hbs.set_strict_mode(true);

        let template = "## Glob\n{{{tools.Glob}}}\n\n## Grep\n{{{tools.Grep}}}";
        let result = hbs
            .render_template(template, &dynamic_values)
            .expect("template should render");

        assert!(result.contains("Use Glob for file patterns."));
        assert!(result.contains("Use Grep for content search."));
    }

    #[tokio::test]
    async fn lazy_partial_resolution_from_db() {
        use crate::AgentOrchestratorBuilder;
        use distri_types::configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig};
        use distri_types::stores::NewPromptTemplate;

        // 1. Create orchestrator with in-memory SQLite
        let db_name = uuid::Uuid::new_v4();
        let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
        let store_config = StoreConfig {
            metadata: MetadataStoreConfig {
                db_config: Some(DbConnectionConfig {
                    database_url: db_url,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let orchestrator = Arc::new(
            AgentOrchestratorBuilder::default()
                .with_store_config(store_config)
                .build()
                .await
                .unwrap(),
        );

        // 2. Store a custom partial in the DB
        let store = orchestrator
            .stores
            .prompt_template_store
            .as_ref()
            .expect("prompt_template_store should exist");

        store
            .create(NewPromptTemplate {
                name: "my_tool_instructions".to_string(),
                template: "Always use Glob before Grep. Read before Edit.".to_string(),
                description: None,
                version: None,
                is_system: false,
            })
            .await
            .unwrap();

        // 3. Verify the partial is NOT pre-registered in the prompt registry
        let registry = orchestrator.get_prompt_registry();
        let known_before = registry.partial_names().await;
        assert!(
            !known_before.contains("my_tool_instructions"),
            "partial should NOT be pre-registered"
        );

        // 4. Build an ExecutorContext that references the orchestrator
        let mut context = ExecutorContext::default();
        context.orchestrator = Some(orchestrator.clone());

        // 5. Render a template that references the DB partial
        let template = "# Instructions\n{{> my_tool_instructions}}\n\nDone.";
        let template_data = crate::agent::prompt_registry::TemplateData::default();

        let result = render_prompt(&Arc::new(context), template, &template_data).await;

        // 6. Verify the partial was lazily resolved and rendered
        let rendered = result.expect("render_prompt should succeed");
        assert!(
            rendered.contains("Always use Glob before Grep"),
            "DB partial content should be in rendered output, got: {}",
            rendered
        );
        assert!(
            rendered.contains("Read before Edit"),
            "full partial content should be rendered"
        );
        assert!(
            rendered.contains("# Instructions"),
            "template structure should be preserved"
        );

        // 7. Verify the partial is now registered (cached for subsequent renders)
        let known_after = registry.partial_names().await;
        assert!(
            known_after.contains("my_tool_instructions"),
            "partial should be registered after first render"
        );
    }

    #[tokio::test]
    async fn lazy_partial_skips_already_registered_builtins() {
        use crate::AgentOrchestratorBuilder;
        use distri_types::configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig};

        let db_name = uuid::Uuid::new_v4();
        let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
        let store_config = StoreConfig {
            metadata: MetadataStoreConfig {
                db_config: Some(DbConnectionConfig {
                    database_url: db_url,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let orchestrator = Arc::new(
            AgentOrchestratorBuilder::default()
                .with_store_config(store_config)
                .build()
                .await
                .unwrap(),
        );

        // "reasoning" is a built-in partial — it should NOT hit the DB
        let registry = orchestrator.get_prompt_registry();
        let known = registry.partial_names().await;
        assert!(
            known.contains("reasoning"),
            "reasoning should be a built-in partial"
        );

        let mut context = ExecutorContext::default();
        context.orchestrator = Some(orchestrator.clone());

        // Template references only built-in partials — should render without DB calls
        let template = "{{> reasoning}}";
        let template_data = crate::agent::prompt_registry::TemplateData::default();

        let result = render_prompt(&Arc::new(context), template, &template_data).await;
        assert!(
            result.is_ok(),
            "rendering built-in partials should work: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn lazy_partial_missing_from_db_fails_in_strict_mode() {
        use crate::AgentOrchestratorBuilder;
        use distri_types::configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig};

        let db_name = uuid::Uuid::new_v4();
        let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
        let store_config = StoreConfig {
            metadata: MetadataStoreConfig {
                db_config: Some(DbConnectionConfig {
                    database_url: db_url,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let orchestrator = Arc::new(
            AgentOrchestratorBuilder::default()
                .with_store_config(store_config)
                .build()
                .await
                .unwrap(),
        );

        let mut context = ExecutorContext::default();
        context.orchestrator = Some(orchestrator);

        // Template references a partial that doesn't exist anywhere
        let template = "{{> totally_nonexistent_partial}}";
        let template_data = crate::agent::prompt_registry::TemplateData::default();

        let result = render_prompt(&Arc::new(context), template, &template_data).await;
        assert!(
            result.is_err(),
            "referencing a nonexistent partial should fail in strict mode"
        );
    }

    // ── load_scratchpad_entries: always task-scoped ────────────────────

    fn task_entry(task_id: &str, parent_task_id: Option<&str>, ts: i64) -> ScratchpadEntry {
        ScratchpadEntry {
            timestamp: ts,
            entry_type: ScratchpadEntryType::Task(vec![distri_types::Part::Text(format!(
                "task {task_id} entry {ts}"
            ))]),
            task_id: task_id.to_string(),
            parent_task_id: parent_task_id.map(str::to_string),
            entry_kind: Some("task".to_string()),
        }
    }

    /// Regression test for the run_skill recursion bug: a top-level
    /// task (no parent_task_id) MUST NOT see SIBLING tasks' scratchpad
    /// entries. Before the fix, the loader fell into a thread-wide
    /// branch when parent_task_id was None and pulled in every entry
    /// in the thread, including sibling tasks' partial reasoning. The
    /// fork's LLM mimicked that reasoning, producing
    /// run_skill→run_skill recursion. The fix removes the branch:
    /// every task — top-level OR subtask — only sees its own entries.
    #[tokio::test]
    async fn load_scratchpad_entries_top_level_excludes_sibling_tasks() {
        let orchestrator = Arc::new(
            crate::AgentOrchestratorBuilder::default()
                .with_store_config(crate::tests::helpers::test_store_config())
                .build()
                .await
                .unwrap(),
        );

        let thread_id = uuid::Uuid::new_v4().to_string();
        let task_a = "task-a".to_string();
        let task_b = "task-b".to_string();

        // Seed: 2 entries for task_a + 3 entries for task_b in the
        // SAME thread. Both are top-level (parent_task_id = None).
        let scratchpad = orchestrator.stores.scratchpad_store.clone();
        for i in 0..2 {
            scratchpad
                .add_entry(&thread_id, task_entry(&task_a, None, i))
                .await
                .expect("seed task_a");
        }
        for i in 0..3 {
            scratchpad
                .add_entry(&thread_id, task_entry(&task_b, None, 100 + i))
                .await
                .expect("seed task_b");
        }

        // Build a context for task_a (top-level: parent_task_id = None).
        let mut context = ExecutorContext::default();
        context.thread_id = thread_id.clone();
        context.task_id = task_a.clone();
        context.parent_task_id = None;
        context.agent_id = "test".to_string();
        context.orchestrator = Some(orchestrator);
        let context = Arc::new(context);

        let agent_def = base_agent_definition(ModelProvider::OpenAI {}, ToolCallFormat::Xml);
        let strategy = AgentStrategy::default();
        let formatter = MessageFormatter::new(&agent_def, &strategy);
        let entries = formatter
            .load_scratchpad_entries(&context, 100)
            .await
            .expect("load");
        assert_eq!(
            entries.len(),
            2,
            "top-level task_a must see ONLY its own 2 entries, not sibling task_b's"
        );
        for e in &entries {
            assert_eq!(e.task_id, task_a, "every entry must belong to task_a");
        }
    }

    /// Symmetric subtask path — already correct pre-fix, pinning it
    /// so a future "optimization" doesn't regress it.
    #[tokio::test]
    async fn load_scratchpad_entries_subtask_is_task_scoped() {
        let orchestrator = Arc::new(
            crate::AgentOrchestratorBuilder::default()
                .with_store_config(crate::tests::helpers::test_store_config())
                .build()
                .await
                .unwrap(),
        );

        let thread_id = uuid::Uuid::new_v4().to_string();
        let parent = "parent-task".to_string();
        let child = "child-task".to_string();
        let other_child = "other-child".to_string();

        let scratchpad = orchestrator.stores.scratchpad_store.clone();
        scratchpad
            .add_entry(&thread_id, task_entry(&parent, None, 0))
            .await
            .unwrap();
        scratchpad
            .add_entry(&thread_id, task_entry(&child, Some(&parent), 1))
            .await
            .unwrap();
        scratchpad
            .add_entry(&thread_id, task_entry(&other_child, Some(&parent), 2))
            .await
            .unwrap();
        scratchpad
            .add_entry(&thread_id, task_entry(&other_child, Some(&parent), 3))
            .await
            .unwrap();

        // Context for `child` — must see only its own 1 entry.
        let mut context = ExecutorContext::default();
        context.thread_id = thread_id;
        context.task_id = child.clone();
        context.parent_task_id = Some(parent.clone());
        context.agent_id = "test".to_string();
        context.orchestrator = Some(orchestrator);
        let context = Arc::new(context);

        let agent_def = base_agent_definition(ModelProvider::OpenAI {}, ToolCallFormat::Xml);
        let strategy = AgentStrategy::default();
        let formatter = MessageFormatter::new(&agent_def, &strategy);
        let entries = formatter
            .load_scratchpad_entries(&context, 100)
            .await
            .unwrap();
        assert_eq!(
            entries.len(),
            1,
            "subtask must see only its own entry, not parent's or sibling's"
        );
        assert_eq!(entries[0].task_id, child);
    }
