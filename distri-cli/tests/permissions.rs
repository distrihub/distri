//! The real local tool handlers, run through an `ExternalToolRegistry` with
//! a scripted broker: permissions decide before anything runs.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use distri::ExternalToolRegistry;
use distri_cli::interact::{Broker, Interaction, Reply};
use distri_cli::local_tools::{register_all, register_approval_handler, ApprovalResult};
use distri_cli::permissions::{PermissionEngine, PermissionMode, PLAN_MODE_REASON};
use distri_types::{AgentEvent, AgentEventType, Part, ToolCall, ToolResponse};
use serde::Deserialize;
use serde_json::json;
use tempfile::TempDir;

/// Answers with the scripted replies in order and records what it was asked.
#[derive(Default)]
struct Scripted {
    replies: Mutex<VecDeque<Reply>>,
    asked: Mutex<Vec<Interaction>>,
}

impl Scripted {
    fn new(replies: impl IntoIterator<Item = Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into_iter().collect()),
            asked: Mutex::default(),
        })
    }

    fn asked(&self) -> Vec<Interaction> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl Broker for Scripted {
    async fn ask(&self, interaction: Interaction) -> Reply {
        self.asked.lock().unwrap().push(interaction);
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("asked more often than scripted")
    }
}

fn event() -> AgentEvent {
    AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: "t".into(),
        run_id: "r".into(),
        task_id: "k".into(),
        parent_task_id: None,
        child: None,
        agent_id: "a".into(),
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
        event: AgentEventType::RunFinished {
            success: true,
            total_steps: 0,
            failed_steps: 0,
            usage: None,
            context_budget: None,
        },
    }
}

async fn call(
    registry: &ExternalToolRegistry,
    tool: &str,
    input: serde_json::Value,
) -> anyhow::Result<ToolResponse> {
    let call = ToolCall {
        tool_call_id: "c1".into(),
        tool_name: tool.into(),
        input,
    };
    registry
        .try_handle("agent", tool, &call, &event())
        .await
        .expect("handler registered")
}

#[derive(Deserialize)]
struct BashOutput {
    stdout: String,
    exit_code: i32,
}

fn data<T: for<'de> Deserialize<'de>>(response: &ToolResponse) -> T {
    match &response.parts[0] {
        Part::Data(v) => serde_json::from_value(v.clone()).unwrap(),
        other => panic!("expected data, got {other:?}"),
    }
}

#[tokio::test]
async fn bash_asks_and_runs_only_when_allowed() {
    let dir = TempDir::new().unwrap();
    let marker = dir.path().join("ran.txt");
    let engine = PermissionEngine::new_for_test(dir.path(), PermissionMode::Default, &[], &[]);
    let broker = Scripted::new([
        Reply::Allow { remember: false },
        Reply::Deny {
            reason: Some("run the tests with nextest".into()),
        },
        Reply::Unavailable,
    ]);
    let registry = ExternalToolRegistry::new();
    register_all(&registry, "agent", dir.path(), engine, broker.clone());

    // Allowed once: the command runs in the project.
    let ok = call(
        &registry,
        "Bash",
        json!({"command": "echo hi > ran.txt && echo done"}),
    )
    .await
    .unwrap();
    let out: BashOutput = data(&ok);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout.trim(), "done");
    assert!(marker.exists());
    std::fs::remove_file(&marker).unwrap();

    // Denied with a reason: nothing runs, the model reads the reason.
    let err = call(&registry, "Bash", json!({"command": "echo hi > ran.txt"}))
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "The user denied this Bash call. They said: run the tests with nextest"
    );
    assert!(!marker.exists());

    // Nobody to ask: refused, with how to allow it.
    let err = call(&registry, "Bash", json!({"command": "cargo test"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.starts_with("Bash needs permission and nobody could be asked (no terminal)."),
        "{err}"
    );
    assert!(err.contains("Bash(cargo test:*)"), "{err}");

    let asked = broker.asked();
    assert_eq!(asked.len(), 3);
    assert_eq!(
        asked[0],
        Interaction::Permit {
            tool: "Bash".into(),
            summary: "echo hi > ran.txt && echo done".into(),
            rule_hint: None,
        }
    );
    assert_eq!(
        asked[2],
        Interaction::Permit {
            tool: "Bash".into(),
            summary: "cargo test".into(),
            rule_hint: Some("Bash(cargo test:*)".into()),
        }
    );
}

#[tokio::test]
async fn remembered_rule_stops_asking_and_deny_rules_never_ask() {
    let dir = TempDir::new().unwrap();
    let engine =
        PermissionEngine::new_for_test(dir.path(), PermissionMode::Default, &[], &["Bash(rm:*)"]);
    let broker = Scripted::new([Reply::Allow { remember: true }]);
    let registry = ExternalToolRegistry::new();
    register_all(&registry, "agent", dir.path(), engine, broker.clone());

    for word in ["one", "two"] {
        let ok = call(
            &registry,
            "Bash",
            json!({"command": format!("echo {word}")}),
        )
        .await
        .unwrap();
        assert_eq!(data::<BashOutput>(&ok).stdout.trim(), word);
    }
    let err = call(&registry, "Bash", json!({"command": "rm -rf ."}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("`Bash(rm:*)`"), "{err}");
    // Asked once: the "always" answer covered the second echo.
    assert_eq!(broker.asked().len(), 1);
}

#[tokio::test]
async fn plan_mode_and_protected_paths_gate_writes() {
    let dir = TempDir::new().unwrap();
    let engine =
        PermissionEngine::new_for_test(dir.path(), PermissionMode::Plan, &["Write(**)"], &[]);
    let broker = Scripted::new([Reply::Deny { reason: None }]);
    let registry = ExternalToolRegistry::new();
    register_all(
        &registry,
        "agent",
        dir.path(),
        engine.clone(),
        broker.clone(),
    );

    let write = |path: &str| json!({"file_path": path, "content": "x"});
    let err = call(&registry, "Write", write("a.txt")).await.unwrap_err();
    assert_eq!(err.to_string(), PLAN_MODE_REASON);
    assert!(!dir.path().join("a.txt").exists());
    // Reads still run in plan mode.
    std::fs::write(dir.path().join("r.txt"), "hello").unwrap();
    assert!(call(&registry, "Read", json!({"file_path": "r.txt"}))
        .await
        .is_ok());

    // Accept edits: the allow rule writes, distri.toml still asks.
    engine.set_mode(PermissionMode::AcceptEdits);
    call(&registry, "Write", write("a.txt")).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "x"
    );
    let err = call(&registry, "Write", write("distri.toml"))
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "The user denied this Write call.");
    assert!(!dir.path().join("distri.toml").exists());
    assert!(matches!(
        &broker.asked()[0],
        Interaction::Permit { tool, rule_hint: None, .. } if tool == "Write"
    ));
}

#[tokio::test]
async fn approval_request_asks_the_broker_and_echoes_the_calls() {
    let broker = Scripted::new([
        Reply::Allow { remember: false },
        Reply::Deny {
            reason: Some("not on Fridays".into()),
        },
        Reply::Unavailable,
    ]);
    let registry = ExternalToolRegistry::new();
    register_approval_handler(&registry, broker.clone());
    let calls = vec![ToolCall {
        tool_call_id: "x".into(),
        tool_name: "deploy".into(),
        input: json!({"env": "prod"}),
    }];
    let input = json!({"tool_calls": calls, "reason": "ships to prod"});

    let yes: ApprovalResult = data(
        &call(&registry, "approval_request", input.clone())
            .await
            .unwrap(),
    );
    assert!(yes.approved);
    assert_eq!(yes.reason, "Approved by user");
    assert_eq!(yes.tool_calls, calls);

    let no: ApprovalResult = data(
        &call(&registry, "approval_request", input.clone())
            .await
            .unwrap(),
    );
    assert!(!no.approved);
    assert_eq!(no.reason, "not on Fridays");

    let nobody: ApprovalResult = data(
        &call(&registry, "approval_request", serde_json::Value::Null)
            .await
            .unwrap(),
    );
    assert!(!nobody.approved);
    assert!(nobody.tool_calls.is_empty());

    assert_eq!(
        broker.asked()[0],
        Interaction::Approve {
            calls,
            reason: Some("ships to prod".into()),
        }
    );
}
