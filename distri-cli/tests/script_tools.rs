//! Script tools through the real `ExternalToolRegistry`: fixture scripts in a temp project,
//! discovered, registered and called with `deno`. Needs `deno` on PATH (or `DISTRI_DENO`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use distri::ExternalToolRegistry;
use distri_cli::interact::{Broker, Interaction, Reply};
use distri_cli::local_events::{LocalEvents, LocalToolEvent, LocalToolEventKind, LogLevel};
use distri_cli::script_tools::{
    self, Permissions, Remembered, ScriptToolError, ScriptToolHost, ScriptToolsConfig, ToolScope,
};
use distri_types::{AgentEvent, AgentEventType, Part, ToolCall};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::sync::broadcast;

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

/// Answers every question with `reply` and counts the questions.
struct TestBroker {
    reply: Reply,
    asked: AtomicUsize,
}

#[async_trait]
impl Broker for TestBroker {
    async fn ask(&self, interaction: Interaction) -> Reply {
        assert!(
            matches!(interaction, Interaction::Grant { .. }),
            "{interaction:?}"
        );
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.reply.clone()
    }
}

fn broker(reply: Reply) -> Arc<TestBroker> {
    Arc::new(TestBroker {
        reply,
        asked: AtomicUsize::new(0),
    })
}

fn config(p: &Project) -> ScriptToolsConfig {
    ScriptToolsConfig {
        project_root: p.root.clone(),
        distri_toml: p.root.join("distri.toml"),
        user_tools_dir: None,
        distri_home: p.home.clone(),
        deno: Some(deno()),
    }
}

fn host_with(p: &Project, broker: Arc<TestBroker>) -> Arc<ScriptToolHost> {
    Arc::new(ScriptToolHost::new(config(p), broker).unwrap())
}

fn host(p: &Project, reply: Reply) -> Arc<ScriptToolHost> {
    host_with(p, broker(reply))
}

const ALWAYS: Reply = Reply::Allow { remember: true };
const ONCE: Reply = Reply::Allow { remember: false };
const NO: Reply = Reply::Deny { reason: None };

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
        child: None,
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
    let host = host(&p, Reply::Unavailable);
    let discovery = host.discover(&reserved()).await;
    assert!(discovery.problems.is_empty(), "{:?}", discovery.problems);

    let registry = ExternalToolRegistry::new();
    let defs = script_tools::register(&host, &registry, &discovery, LocalEvents::default());
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
    let host = host(&p, ALWAYS);
    let discovery = host.discover(&reserved()).await;
    let registry = ExternalToolRegistry::new();
    script_tools::register(&host, &registry, &discovery, LocalEvents::default());

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
    let denied = host(&p, NO);
    let discovery = denied.discover(&reserved()).await;
    let tool = discovery.find("read_file").unwrap().clone();
    let err = denied
        .call(&tool, &serde_json::json!({"path": "data/note.txt"}), None)
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
    let would_allow = host(&p, ALWAYS);
    assert!(would_allow
        .call(&tool, &serde_json::json!({"path": "data/note.txt"}), None)
        .await
        .is_err());

    // distri.toml grants it without asking.
    std::fs::write(
        p.root.join("distri.toml"),
        "[tools.read_file]\nread = [\"./data\"]\n",
    )
    .unwrap();
    let out = host(&p, Reply::Unavailable)
        .call(&tool, &serde_json::json!({"path": "data/note.txt"}), None)
        .await
        .unwrap();
    assert_eq!(out.value["text"], "inside the grant");
}

#[tokio::test]
async fn env_is_never_granted() {
    let p = project(&["peek_env.ts"]);
    let host = host(&p, ALWAYS);
    let discovery = host.discover(&reserved()).await;
    let tool = discovery.find("peek_env").unwrap();
    let msg = host
        .call(tool, &serde_json::json!({}), None)
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
    let host = host(&p, Reply::Unavailable);
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
    let host = ScriptToolHost::new(cfg, broker(Reply::Unavailable)).unwrap();
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
    let host = ScriptToolHost::new(cfg, broker(Reply::Unavailable)).unwrap();
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
    // The template's ctx.log is printed as it arrives.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("working on hello"), "{stderr}");

    // Input that does not match the schema never reaches the script.
    let bad = run(&["tools", "test", "greet", r#"{"input": 5}"#]);
    assert!(!bad.status.success());
    let stderr = String::from_utf8_lossy(&bad.stderr);
    assert!(
        stderr.contains(
            "script tool 'greet': input does not match its schema: /input: 5 is not of type \"string\""
        ),
        "{stderr}"
    );

    // `tools check`: one line per tool, non-zero when any fails.
    let ok = run(&["tools", "check"]);
    let stdout = String::from_utf8_lossy(&ok.stdout);
    assert!(ok.status.success(), "{ok:?}");
    assert!(stdout.contains("greet  deno check · schema ok"), "{stdout}");

    let tools = p.root.join(".distri/tools");
    std::fs::copy(
        Path::new(FIXTURES).join("wrong_types.ts"),
        tools.join("wrong_types.ts"),
    )
    .unwrap();
    let failed = run(&["tools", "check"]);
    let stdout = String::from_utf8_lossy(&failed.stdout);
    assert_eq!(failed.status.code(), Some(1), "{failed:?}");
    assert!(stdout.contains("greet  deno check · schema ok"), "{stdout}");
    assert!(stdout.contains("✗"), "{stdout}");
    assert!(stdout.contains("wrong_types  "), "{stdout}");
    // The first lines of the type error (an object literal against `defineTool`'s overloads).
    assert!(stdout.contains("TS2769 [ERROR]"), "{stdout}");
    // Only the named one.
    let one = run(&["tools", "check", "greet"]);
    assert!(one.status.success(), "{one:?}");
    assert!(!String::from_utf8_lossy(&one.stdout).contains("wrong_types"));
}

fn events_now(rx: &mut broadcast::Receiver<LocalToolEvent>) -> Vec<LocalToolEvent> {
    let mut out = Vec::new();
    while let Ok(e) = rx.try_recv() {
        out.push(e);
    }
    out
}

/// Wait (at most `within`) for an event matching `want`.
async fn wait_for(
    rx: &mut broadcast::Receiver<LocalToolEvent>,
    within: Duration,
    want: impl Fn(&LocalToolEventKind) -> bool,
) -> Option<LocalToolEvent> {
    tokio::time::timeout(within, async {
        loop {
            match rx.recv().await {
                Ok(e) if want(&e.kind) => return Some(e),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

fn log(level: LogLevel, line: &str) -> LocalToolEventKind {
    LocalToolEventKind::Log {
        level,
        line: line.into(),
    }
}

#[tokio::test]
async fn a_typed_tool_streams_progress_and_logs_keyed_by_the_call() {
    let p = project(&["typed_pages.ts"]);
    let host = host(&p, Reply::Unavailable);
    let discovery = host.discover(&reserved()).await;
    assert!(discovery.problems.is_empty(), "{:?}", discovery.problems);
    let events = LocalEvents::default();
    let mut rx = events.subscribe();
    let registry = ExternalToolRegistry::new();
    let defs = script_tools::register(&host, &registry, &discovery, events.clone());
    // `input: s.object(...)` arrives as the v1 `parameters` the engine reads.
    assert_eq!(defs[0].parameters["required"], serde_json::json!(["pages"]));
    assert_eq!(defs[0].parameters["properties"]["pages"]["type"], "integer");

    let resp = registry
        .try_handle(
            "a",
            "typed_pages",
            &call("typed_pages", serde_json::json!({"pages": 2})),
            &event(),
        )
        .await
        .unwrap()
        .expect("call succeeds");
    let out: serde_json::Value = data_part(&resp.parts);
    assert_eq!(out["done"], 2);
    assert_eq!(out["cwd"], p.root.display().to_string());

    let got = events_now(&mut rx);
    assert!(got
        .iter()
        .all(|e| e.tool_call_id == "tc-1" && e.tool_name == "typed_pages"));
    let kinds: Vec<LocalToolEventKind> = got.into_iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        vec![
            log(LogLevel::Log, "starting typed_pages"),
            LocalToolEventKind::Progress {
                fraction: Some(0.5),
                message: Some("page 1/2".into())
            },
            LocalToolEventKind::Progress {
                fraction: Some(1.0),
                message: Some("page 2/2".into())
            },
            log(LogLevel::Warn, "careful"),
            log(LogLevel::Log, "plain stderr"),
        ]
    );
}

#[tokio::test]
async fn input_that_does_not_match_the_schema_never_starts_deno() {
    let p = project(&["slow_strict.ts"]);
    let host = host(&p, Reply::Unavailable);
    let discovery = host.discover(&reserved()).await;
    let tool = discovery.find("slow_strict").unwrap();

    let started = Instant::now();
    let err = host
        .call(tool, &serde_json::json!({"id": 17}), None)
        .await
        .unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert!(matches!(err, ScriptToolError::InvalidInput { .. }));
    assert_eq!(
        err.to_string(),
        "script tool 'slow_strict': input does not match its schema: /id: 17 is not of type \"string\""
    );
    let err = host
        .call(tool, &serde_json::json!({"n": 0}), None)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("\"id\" is a required property"), "{err}");
    assert!(err.contains("/n: 0 is less than the minimum of 1"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn cancelling_a_call_sends_sigterm_so_ctx_signal_aborts() {
    let p = project(&["typed_pages.ts"]);
    let host = host(&p, Reply::Unavailable);
    let discovery = host.discover(&reserved()).await;
    let events = LocalEvents::default();
    let mut rx = events.subscribe();
    let registry = ExternalToolRegistry::new();
    script_tools::register(&host, &registry, &discovery, events);

    let c = call("typed_pages", serde_json::json!({"pages": 1, "hang": true}));
    let ev = event();
    let running = registry.try_handle("a", "typed_pages", &c, &ev);
    tokio::select! {
        r = running => panic!("the call should hang until cancelled: {r:?}"),
        seen = wait_for(&mut rx, Duration::from_secs(20), |k| *k == log(LogLevel::Warn, "careful")) => {
            assert!(seen.is_some(), "the script never started");
        }
    }
    // `running` is dropped here: the run was cancelled.
    let aborted = wait_for(&mut rx, Duration::from_secs(4), |k| {
        *k == log(LogLevel::Warn, "aborted")
    })
    .await
    .expect("SIGTERM reached the script and ctx.signal aborted");
    assert_eq!(aborted.tool_call_id, "tc-1");
}

#[cfg(unix)]
#[tokio::test]
async fn a_script_that_ignores_the_cancel_is_killed_after_the_grace_period() {
    let p = project(&["ignores_term.ts"]);
    let host = host(&p, Reply::Unavailable);
    let discovery = host.discover(&reserved()).await;
    let tool = discovery.find("ignores_term").unwrap().clone();
    let (tx, mut pids) = tokio::sync::mpsc::unbounded_channel::<i32>();
    let sink: script_tools::EventSink = Arc::new(move |kind| {
        if let LocalToolEventKind::Log { line, .. } = kind {
            if let Some(pid) = line.strip_prefix("pid ").and_then(|p| p.parse().ok()) {
                let _ = tx.send(pid);
            }
        }
    });
    let input = serde_json::json!({});
    let running = host.call(&tool, &input, Some(sink));
    let pid = tokio::select! {
        r = running => panic!("the call should hang: {r:?}"),
        pid = pids.recv() => pid.expect("the script logs its pid"),
    };
    let alive = |pid: i32| unsafe { libc::kill(pid, 0) } == 0;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(alive(pid), "SIGTERM alone does not stop it");
    let deadline = Instant::now() + Duration::from_secs(4);
    while alive(pid) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!alive(pid), "SIGKILL after the grace period");
}

#[tokio::test]
async fn timeouts_come_from_the_frontmatter_or_distri_toml() {
    let p = project(&["sleepy.ts", "lookup_order.ts"]);
    let host = host(&p, Reply::Unavailable);
    let discovery = host.discover(&reserved()).await;
    let sleepy = discovery.find("sleepy").unwrap();
    let lookup = discovery.find("lookup_order").unwrap();
    assert_eq!(host.timeout_for(lookup).unwrap(), Duration::from_secs(120));
    assert_eq!(host.timeout_for(sleepy).unwrap(), Duration::from_secs(1));

    let started = Instant::now();
    let err = host
        .call(sleepy, &serde_json::json!({}), None)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "script tool 'sleepy' timed out after 1s");
    assert!(started.elapsed() < Duration::from_secs(5));

    std::fs::write(
        p.root.join("distri.toml"),
        "[tools.sleepy]\ntimeout = 7\n[tools.lookup_order]\ntimeout = 30\n",
    )
    .unwrap();
    assert_eq!(host.timeout_for(sleepy).unwrap(), Duration::from_secs(7));
    assert_eq!(host.timeout_for(lookup).unwrap(), Duration::from_secs(30));
}

#[tokio::test]
async fn grants_go_through_the_broker_once_or_always() {
    let p = project(&["read_file.ts"]);
    let request = Permissions {
        net: BTreeSet::new(),
        read: BTreeSet::from(["./data".to_string()]),
    };
    let input = serde_json::json!({"path": "data/note.txt"});

    // "Once": this call runs with the grant; nothing is stored, so the next call asks again.
    let once = broker(ONCE);
    let h = host_with(&p, once.clone());
    let discovery = h.discover(&reserved()).await;
    let tool = discovery.find("read_file").unwrap().clone();
    assert_eq!(
        h.call(&tool, &input, None).await.unwrap().value["text"],
        "inside the grant"
    );
    assert_eq!(
        h.grants()
            .lookup(&p.root, "read_file", &tool.path, &request)
            .unwrap(),
        Remembered::Unknown
    );
    h.call(&tool, &input, None).await.unwrap();
    assert_eq!(once.asked.load(Ordering::SeqCst), 2);

    // Nobody to ask: denied for this call, not remembered.
    let nobody = broker(Reply::Unavailable);
    let h = host_with(&p, nobody.clone());
    assert!(h
        .call(&tool, &input, None)
        .await
        .unwrap_err()
        .to_string()
        .starts_with("NotCapable"));
    assert_eq!(
        h.grants()
            .lookup(&p.root, "read_file", &tool.path, &request)
            .unwrap(),
        Remembered::Unknown
    );

    // "Always": stored; later calls (any broker) do not ask.
    let always = broker(ALWAYS);
    host_with(&p, always.clone())
        .call(&tool, &input, None)
        .await
        .unwrap();
    assert_eq!(always.asked.load(Ordering::SeqCst), 1);
    let after = broker(Reply::Unavailable);
    let h = host_with(&p, after.clone());
    assert_eq!(
        h.grants()
            .lookup(&p.root, "read_file", &tool.path, &request)
            .unwrap(),
        Remembered::Allowed
    );
    h.call(&tool, &input, None).await.unwrap();
    assert_eq!(after.asked.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn extraction_failures_are_cached_and_sibling_edits_re_extract() {
    let p = project(&["uses_lib.ts", "greeting_lib.ts", "broken.ts"]);
    let host = host(&p, Reply::Unavailable);
    let first = host.discover(&reserved()).await;
    assert_eq!(first.find("uses_lib").unwrap().export.description, "first");
    let broken = |d: &script_tools::Discovery| {
        d.problems
            .iter()
            .find(|pr| pr.path.ends_with("broken.ts"))
            .cloned()
            .expect("broken.ts is a problem")
    };
    let fresh = broken(&first);
    assert!(!fresh.cached);
    assert!(
        fresh.message.contains("broken at import"),
        "{}",
        fresh.message
    );

    // Unchanged: the failure comes from the cache, short, without running the import again.
    let again = host.discover(&reserved()).await;
    let cached = broken(&again);
    assert!(cached.cached);
    assert_eq!(cached.message, fresh.message);
    assert!(cached.summary().contains("broken at import"));

    // Editing the imported sibling re-extracts the tool that imports it.
    let tools = p.root.join(".distri/tools");
    std::fs::write(
        tools.join("greeting_lib.ts"),
        "export const DESCRIPTION = \"second\";\n",
    )
    .unwrap();
    let edited = host.discover(&reserved()).await;
    assert_eq!(
        edited.find("uses_lib").unwrap().export.description,
        "second"
    );

    // A sibling that breaks: a fresh failure first, a cached one after.
    std::fs::write(
        tools.join("greeting_lib.ts"),
        "export const DESCRIPTION: string = (() => { throw new Error(\"lib broke\"); })();\n",
    )
    .unwrap();
    let broke = host.discover(&reserved()).await;
    let pr = broke
        .problems
        .iter()
        .find(|pr| pr.path.ends_with("uses_lib.ts"))
        .expect("uses_lib fails to import");
    assert!(pr.message.contains("lib broke") && !pr.cached, "{pr:?}");
    let still = host.discover(&reserved()).await;
    assert!(still
        .problems
        .iter()
        .any(|pr| pr.path.ends_with("uses_lib.ts") && pr.cached));

    // Fixing the script itself clears its cached failure.
    std::fs::write(
        tools.join("broken.ts"),
        "import { defineTool } from \"jsr:@distri/tool\";\nexport const tool = defineTool({ name: \"fixed\" });\nexport default () => ({});\n",
    )
    .unwrap();
    std::fs::write(
        tools.join("greeting_lib.ts"),
        "export const DESCRIPTION = \"third\";\n",
    )
    .unwrap();
    let fixed = host.discover(&reserved()).await;
    assert!(fixed.problems.is_empty(), "{:?}", fixed.problems);
    assert!(fixed.find("fixed").is_some());
}
