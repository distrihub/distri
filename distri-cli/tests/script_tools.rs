//! Script tools through the real `ExternalToolRegistry`: fixture scripts in a temp project,
//! discovered, registered and called with `deno`. Needs `deno` on PATH (or `DISTRI_DENO`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use distri::ExternalToolRegistry;
use distri_cli::script_tools::{
    self, FixedApprover, Permissions, Remembered, ScriptToolHost, ScriptToolsConfig, ToolScope,
};
use distri_types::{AgentEvent, AgentEventType, Part, ToolCall};
use serde::Deserialize;
use tempfile::TempDir;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/script_tools");

fn deno() -> PathBuf {
    script_tools::runner::locate_deno()
        .expect("these tests need `deno` on PATH (or DISTRI_DENO): https://deno.com")
}

struct Project {
    _dir: TempDir,
    root: PathBuf,
    home: PathBuf,
}

/// A project with the named fixtures in `.distri/tools/` and a `data/` folder.
fn project(fixtures: &[&str]) -> Project {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap().join("project");
    let home = dir.path().canonicalize().unwrap().join("home");
    let tools = root.join(".distri").join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::write(root.join("data").join("note.txt"), "inside the grant").unwrap();
    std::fs::write(root.join("secret.txt"), "outside the grant").unwrap();
    for f in fixtures {
        std::fs::copy(Path::new(FIXTURES).join(f), tools.join(f)).unwrap();
    }
    Project {
        _dir: dir,
        root,
        home,
    }
}

fn host(p: &Project, approve: Option<bool>) -> Arc<ScriptToolHost> {
    let cfg = ScriptToolsConfig {
        project_root: p.root.clone(),
        distri_toml: p.root.join("distri.toml"),
        user_tools_dir: None,
        distri_home: p.home.clone(),
        deno: Some(deno()),
    };
    Arc::new(ScriptToolHost::new(cfg, Arc::new(FixedApprover(approve))).unwrap())
}

fn reserved() -> BTreeSet<String> {
    BTreeSet::from(["Bash".to_string()])
}

fn event() -> AgentEvent {
    AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: "t".into(),
        run_id: "r".into(),
        task_id: "k".into(),
        parent_task_id: None,
        agent_id: "order_agent".into(),
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

fn call(tool: &str, input: serde_json::Value) -> ToolCall {
    ToolCall {
        tool_call_id: "tc-1".into(),
        tool_name: tool.into(),
        input,
    }
}

#[derive(Debug, Deserialize, PartialEq)]
struct Order {
    id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct FileText {
    text: String,
}

fn data_part<T: serde::de::DeserializeOwned>(parts: &[Part]) -> T {
    match parts {
        [Part::Data(v)] => serde_json::from_value(v.clone()).unwrap(),
        other => panic!("expected one data part, got {other:?}"),
    }
}

#[tokio::test]
async fn a_script_is_registered_and_called_through_the_registry() {
    let p = project(&["lookup_order.ts"]);
    let host = host(&p, None);
    let discovery = host.discover(&reserved()).await;
    assert!(discovery.problems.is_empty(), "{:?}", discovery.problems);

    let registry = ExternalToolRegistry::new();
    let defs = script_tools::register(&host, &registry, &discovery);
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name, "lookup_order");
    assert_eq!(defs[0].description, "Fetch an order by id");
    assert_eq!(defs[0].parameters["required"][0], "id");
    // Registered under "*": every agent sees it, like the built-ins.
    assert!(registry.has_tool("order_agent", "lookup_order"));

    let resp = registry
        .try_handle(
            "order_agent",
            "lookup_order",
            &call("lookup_order", serde_json::json!({"id": "A-17"})),
            &event(),
        )
        .await
        .expect("handler registered")
        .expect("call succeeds");
    assert_eq!(resp.tool_name, "lookup_order");
    assert_eq!(
        data_part::<Order>(&resp.parts),
        Order {
            id: "A-17".into(),
            status: "shipped".into()
        }
    );
}

#[tokio::test]
async fn a_read_outside_the_grant_is_not_capable_and_names_the_permission() {
    let p = project(&["read_file.ts"]);
    // The frontmatter asks for ./data; approve it.
    let host = host(&p, Some(true));
    let discovery = host.discover(&reserved()).await;
    let registry = ExternalToolRegistry::new();
    script_tools::register(&host, &registry, &discovery);

    let inside = registry
        .try_handle(
            "a",
            "read_file",
            &call("read_file", serde_json::json!({"path": "data/note.txt"})),
            &event(),
        )
        .await
        .unwrap()
        .expect("granted read works");
    assert_eq!(
        data_part::<FileText>(&inside.parts).text,
        "inside the grant"
    );

    let err = registry
        .try_handle(
            "a",
            "read_file",
            &call("read_file", serde_json::json!({"path": "secret.txt"})),
            &event(),
        )
        .await
        .unwrap()
        .expect_err("read outside the grant fails");
    let msg = err.to_string();
    assert!(
        msg.starts_with("NotCapable: script tool 'read_file' needs read access to"),
        "{msg}"
    );
    assert!(msg.contains("secret.txt"), "{msg}");

    // The registry is still usable after a failed call: the loop continues.
    assert!(registry
        .try_handle(
            "a",
            "read_file",
            &call("read_file", serde_json::json!({"path": "data/note.txt"})),
            &event(),
        )
        .await
        .unwrap()
        .is_ok());
}

#[tokio::test]
async fn frontmatter_requests_are_asked_once_and_remembered_per_project() {
    let p = project(&["read_file.ts"]);
    let denied = host(&p, Some(false));
    let discovery = denied.discover(&reserved()).await;
    let tool = discovery.find("read_file").unwrap().clone();
    let err = denied
        .call(&tool, &serde_json::json!({"path": "data/note.txt"}))
        .await
        .unwrap_err();
    assert!(err.to_string().starts_with("NotCapable"), "{err}");
    let request = Permissions {
        net: BTreeSet::new(),
        read: BTreeSet::from(["./data".to_string()]),
    };
    assert_eq!(
        denied
            .grants()
            .lookup(&p.root, "read_file", &tool.path, &request)
            .unwrap(),
        Remembered::Denied
    );

    // The remembered "no" holds even for an approver that would say yes.
    let would_allow = host(&p, Some(true));
    assert!(would_allow
        .call(&tool, &serde_json::json!({"path": "data/note.txt"}))
        .await
        .is_err());

    // distri.toml grants it without asking.
    std::fs::write(
        p.root.join("distri.toml"),
        "[tools.read_file]\nread = [\"./data\"]\n",
    )
    .unwrap();
    let out = host(&p, None)
        .call(&tool, &serde_json::json!({"path": "data/note.txt"}))
        .await
        .unwrap();
    assert_eq!(out.value["text"], "inside the grant");
}

#[tokio::test]
async fn env_is_never_granted() {
    let p = project(&["peek_env.ts"]);
    let host = host(&p, Some(true));
    let discovery = host.discover(&reserved()).await;
    let tool = discovery.find("peek_env").unwrap();
    let msg = host
        .call(tool, &serde_json::json!({}))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        msg.starts_with("NotCapable: script tool 'peek_env' needs env access"),
        "{msg}"
    );
    assert!(msg.contains("never granted env access"), "{msg}");
}

#[tokio::test]
async fn discovery_reports_load_errors_jobs_and_builtin_clashes() {
    let p = project(&["lookup_order.ts", "broken.ts", "a_job.ts", "bash_clash.ts"]);
    let host = host(&p, None);
    let discovery = host.discover(&reserved()).await;

    let names: Vec<&str> = discovery.tools.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["lookup_order"]);
    assert_eq!(discovery.tools[0].scope, ToolScope::Project);
    assert!(discovery.tools[0]
        .path
        .ends_with(".distri/tools/lookup_order.ts"));
    assert_eq!(discovery.not_tools.len(), 1);
    assert!(discovery.not_tools[0].ends_with("a_job.ts"));

    let problem = |file: &str| {
        discovery
            .problems
            .iter()
            .find(|pr| pr.path.ends_with(file))
            .unwrap_or_else(|| panic!("no problem for {file}: {:?}", discovery.problems))
            .message
            .clone()
    };
    assert!(problem("broken.ts").contains("broken at import"));
    assert!(problem("bash_clash.ts").contains("conflicts with a built-in"));

    // The second scan comes from the cache and says the same.
    let again = host.discover(&reserved()).await;
    assert_eq!(again.tools.len(), 1);
    assert_eq!(again.not_tools.len(), 1);
}

#[tokio::test]
async fn project_tools_shadow_user_tools() {
    let p = project(&["lookup_order.ts"]);
    let user_dir = p.home.join("tools");
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::copy(
        Path::new(FIXTURES).join("lookup_order.ts"),
        user_dir.join("lookup_order.ts"),
    )
    .unwrap();
    let cfg = ScriptToolsConfig {
        project_root: p.root.clone(),
        distri_toml: p.root.join("distri.toml"),
        user_tools_dir: Some(user_dir.canonicalize().unwrap()),
        distri_home: p.home.clone(),
        deno: Some(deno()),
    };
    let host = ScriptToolHost::new(cfg, Arc::new(FixedApprover(None))).unwrap();
    let discovery = host.discover(&reserved()).await;
    assert_eq!(discovery.tools.len(), 1);
    assert_eq!(discovery.tools[0].scope, ToolScope::Project);
    assert!(discovery.problems[0].message.contains("shadowed"));
}

#[tokio::test]
async fn without_deno_the_scripts_are_named_as_skipped() {
    let p = project(&["lookup_order.ts"]);
    let cfg = ScriptToolsConfig {
        project_root: p.root.clone(),
        distri_toml: p.root.join("distri.toml"),
        user_tools_dir: None,
        distri_home: p.home.clone(),
        deno: None,
    };
    let host = ScriptToolHost::new(cfg, Arc::new(FixedApprover(None))).unwrap();
    let discovery = host.discover(&reserved()).await;
    assert!(discovery.tools.is_empty());
    let notice = discovery.deno_missing_notice().unwrap();
    assert!(notice.contains("Deno is not installed"), "{notice}");
    assert!(notice.contains("lookup_order.ts"), "{notice}");
}

#[test]
fn the_cli_scaffolds_and_tests_a_tool() {
    let p = project(&[]);
    let bin = env!("CARGO_BIN_EXE_distri");
    let run = |args: &[&str]| {
        std::process::Command::new(bin)
            .args(args)
            .current_dir(&p.root)
            .env("HOME", &p.home)
            .env("DISTRI_DENO", deno())
            .env("DISTRI_LOG", "error")
            .output()
            .unwrap()
    };
    let created = run(&["tools", "new", "greet"]);
    assert!(created.status.success(), "{created:?}");
    assert!(p.root.join(".distri/tools/greet.ts").is_file());

    let out = run(&["tools", "test", "greet", r#"{"input": "hello"}"#]);
    assert!(out.status.success(), "{out:?}");
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["input"], "hello");
    assert_eq!(value["ok"], true);
}
