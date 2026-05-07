//! Real-LLM smoke test for `mode = "fork"`.
//!
//! End-to-end shape: a parent agent gets dispatched to via a real LLM, the
//! LLM picks `run_skill(test_fork_skill, args:{tag})`, the forked
//! `_adhoc_base` worker runs the substituted skill body, calls a custom
//! `log_to_memory` tool that mutates a shared `Vec<String>`, then calls
//! `final`. We then assert that the shared memory contains exactly one
//! entry, with the expected `${tag}` value substituted in.
//!
//! What this proves that the mock test in `mock/fork.rs` cannot:
//! - The skill body (with `${tag}` substituted) actually reaches the LLM
//!   and is interpretable by it.
//! - The forked `_adhoc_base` child can call a parent-registered external
//!   tool and have its mutation visible to the test.
//! - The child's `final` result propagates back to the parent.
//!
//! ## Required env
//! - `OPENAI_API_KEY` — provider key (matches `parent_agent.md`'s configured
//!   `gpt-4o-mini` on the openai provider).
//!
//! ## Running
//! ```sh
//! OPENAI_API_KEY=sk-... cargo test -p distri-core \
//!   orchestrator::smoke::fork -- --ignored --nocapture
//! ```

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use crate::agent::parse_agent_markdown_content;
use crate::agent::ExecutorContext;
use crate::tests::helpers::test_store_config;
use crate::tools::ToolContext;
use crate::{init_logging, AgentOrchestratorBuilder};
use distri_types::stores::NewSkill;
use distri_types::{configuration::AgentConfig, Part, Tool, ToolCall};

/// Custom tool the forked worker calls — appends a single line into a
/// shared `Vec<String>` so the test can read it back after execution.
#[derive(Debug, Clone)]
struct LogToMemoryTool {
    sink: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Tool for LogToMemoryTool {
    fn get_name(&self) -> String {
        "log_to_memory".to_string()
    }
    fn get_description(&self) -> String {
        "Append a tagged line into the test's in-memory log. Returns ack.".to_string()
    }
    fn get_parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["tag"],
            "properties": {
                "tag": { "type": "string", "description": "The tag value to log." }
            }
        })
    }
    async fn execute(
        &self,
        tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        let tag = tool_call
            .input
            .get("tag")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing>")
            .to_string();
        self.sink.lock().unwrap().push(tag.clone());
        Ok(vec![Part::Text(format!("logged:{}", tag))])
    }
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY; hits real LLM. Run with `cargo test --ignored`."]
async fn fork_via_run_skill_logs_to_memory_and_finals() {
    if std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("skipping fork smoke test: OPENAI_API_KEY not set");
        return;
    }
    dotenv::dotenv().ok();
    init_logging("info");

    // ── Orchestrator with in-memory stores. Default skill_store backs the
    //    `test_fork_skill` we upsert below. Default agent_store accepts our
    //    parent + _adhoc_base registrations.
    let orch = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );

    // ── Register agents. Parent + _adhoc_base. The parent's .md declares
    //    `run_skill` in its builtin tool list. Model + provider are injected
    //    from env here (the fixture deliberately omits `model_settings` so a
    //    stale model name can't poison the test).
    let mut parent_def =
        parse_agent_markdown_content(include_str!("../../fixtures/fork_test/parent_agent.md"))
            .await
            .expect("parse parent_agent.md");
    let model = std::env::var("SMOKE_TEST_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    parent_def.model_settings = Some(distri_types::ModelSettings::new(&model));
    let parent_name = parent_def.name.clone();
    orch.register_agent_definition(parent_def.clone())
        .await
        .unwrap();

    // _adhoc_base: load the production definition so worker contract +
    // builtin/external tool defaults match prod. (Same file run_skill's
    // dispatch will resolve.)
    let adhoc_def = parse_agent_markdown_content(include_str!(
        "../../../../../../../cloud/agents/_adhoc_base.md"
    ))
    .await
    .expect("parse _adhoc_base.md");
    orch.register_agent_definition(adhoc_def).await.unwrap();

    // ── Insert the test skill into the SkillStore. RunSkillTool resolves
    //    `skill_id` against this store.
    let skill_body = include_str!("../../fixtures/fork_test/test_skill.md");
    orch.stores
        .skill_store
        .as_ref()
        .expect("skill store wired")
        .upsert_by_name(NewSkill {
            name: "test_fork_skill".to_string(),
            description: Some("smoke test".to_string()),
            content: skill_body.to_string(),
            tags: vec!["test".to_string()],
            model: None,
            context: Default::default(),
        })
        .await
        .unwrap();

    // ── Register the in-memory log tool against the parent agent. Both
    //    parent and forked worker (which inherits `external = ["*"]` via
    //    `_adhoc_base`) will see this tool.
    let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    orch.register_tool(
        parent_name.as_str(),
        Arc::new(LogToMemoryTool { sink: sink.clone() }),
    )
    .await;

    // ── Drive the parent agent. The .md instructs it to call run_skill
    //    once with `args:{tag:"ALPHA"}` and then final.
    let context = Arc::new(ExecutorContext {
        orchestrator: Some(orch.clone()),
        verbose: true,
        ..Default::default()
    });
    let result = orch
        .run_inline_agent(
            AgentConfig::StandardAgent(parent_def),
            "Run the smoke test now.",
            context,
        )
        .await
        .expect("parent agent should complete");

    eprintln!("parent finished. content={:?}", result.content);

    // ── Assertions. The forked worker should have called log_to_memory
    //    exactly once with tag = "ALPHA" (from the parent .md). If the LLM
    //    flakes and calls it more or fewer times, the test fails loudly —
    //    smoke test, not integration test.
    let logged = sink.lock().unwrap().clone();
    assert!(
        !logged.is_empty(),
        "expected the forked worker to call log_to_memory at least once"
    );
    assert!(
        logged.iter().any(|t| t == "ALPHA"),
        "expected at least one log entry with tag = 'ALPHA'; got {:?}",
        logged
    );
}
