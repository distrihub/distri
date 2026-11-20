//! Conformance: what distri-a2a serializes validates against the official
//! A2A 1.0 JSON Schema bundle (`schema/a2a.json`, vendored byte-identical from
//! a2aproject/a2a-rs `a2acli/schema/a2a.json`), and round-trips.

use std::collections::{BTreeMap, HashMap};

use chrono::TimeZone;
use distri_a2a::*;
use serde_json::{json, Map, Value};

const BUNDLE: &str = include_str!("../schema/a2a.json");

fn normalise(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Rewrite every `$ref` (internal `#/definitions/Agent Card` or external
/// `a2a.v1.AgentSkill.jsonschema.json`) to a normalised definition key, the
/// same way a2acli's `card get --validate` does. Fails on a ref it cannot
/// place, so no subtree goes unvalidated.
fn retarget(node: &mut Value, index: &HashMap<String, String>, unresolved: &mut Vec<String>) {
    match node {
        Value::Object(map) => {
            if let Some(Value::String(r)) = map.get("$ref") {
                let name = r.strip_prefix("#/definitions/").unwrap_or(r);
                let base = name.strip_suffix(".jsonschema.json").unwrap_or(name);
                let key = normalise(base.rsplit('.').next().unwrap_or(base));
                if index.contains_key(&key) {
                    map.insert("$ref".into(), Value::String(format!("#/definitions/{key}")));
                } else {
                    unresolved.push(r.clone());
                }
            }
            for v in map.values_mut() {
                retarget(v, index, unresolved);
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|v| retarget(v, index, unresolved)),
        _ => {}
    }
}

fn schema_for(definition: &str) -> Value {
    let mut bundle: Value = serde_json::from_str(BUNDLE).unwrap();
    let index: HashMap<String, String> = bundle["definitions"]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| (normalise(k), k.clone()))
        .collect();
    let key = normalise(definition);
    assert!(index.contains_key(&key), "no definition {definition}");
    let mut unresolved = Vec::new();
    retarget(&mut bundle, &index, &mut unresolved);
    assert!(unresolved.is_empty(), "unplaceable refs: {unresolved:?}");
    let defs: Map<String, Value> = bundle["definitions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (normalise(k), v.clone()))
        .collect();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": format!("#/definitions/{key}"),
        "definitions": defs,
    })
}

#[track_caller]
fn assert_valid(definition: &str, instance: &Value) {
    let schema = schema_for(definition);
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{definition} invalid:\n{}\n{}",
        errors.join("\n"),
        serde_json::to_string_pretty(instance).unwrap()
    );
}

/// Validate, then check it decodes back to the same JSON.
#[track_caller]
fn conforms<T: serde::Serialize + serde::de::DeserializeOwned>(definition: &str, value: &T) {
    let json = serde_json::to_value(value).unwrap();
    assert_valid(definition, &json);
    let back: T = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&back).unwrap(),
        json,
        "{definition} round-trip"
    );
}

fn meta(pairs: &[(&str, Value)]) -> Metadata {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn all_parts() -> Vec<Part> {
    vec![
        Part::text("hello"),
        Part::text("with meta").with_metadata("k", json!(1)),
        Part::raw(vec![1, 2, 3])
            .with_media_type("image/png")
            .with_filename("a.png"),
        Part::url("https://example.com/a.pdf")
            .with_media_type("application/pdf")
            .with_filename("a.pdf"),
        Part::data(json!({"plain": true})),
        Part::data(json!({"tool_call_id": "c1", "tool_name": "search", "input": {}}))
            .with_metadata(legacy::PART_TYPE_KEY, json!("tool_call")),
    ]
}

fn message() -> Message {
    Message {
        message_id: "m1".into(),
        context_id: Some("c1".into()),
        task_id: Some("t1".into()),
        role: Role::Agent,
        parts: all_parts(),
        metadata: Some(meta(&[("is_final", json!(true))])),
        extensions: vec!["https://distri.dev/a2a/extensions/event-stream/v1".into()],
        reference_task_ids: vec!["t0".into()],
    }
}

fn ts() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc
        .with_ymd_and_hms(2026, 11, 20, 12, 0, 0)
        .unwrap()
}

fn artifact() -> Artifact {
    Artifact {
        artifact_id: "a1".into(),
        name: Some("report.md".into()),
        description: Some("the report".into()),
        parts: vec![Part::text("# Report")],
        metadata: Some(meta(&[("k", json!("v"))])),
        extensions: vec!["urn:x".into()],
    }
}

fn task(state: TaskState) -> Task {
    Task {
        id: "t1".into(),
        context_id: "c1".into(),
        status: TaskStatus {
            state,
            message: Some(message()),
            timestamp: Some(ts()),
        },
        artifacts: vec![artifact()],
        history: vec![Message::new(Role::User, vec![Part::text("hi")])],
        metadata: Some(meta(&[("pending_calls", json!([]))])),
    }
}

fn card() -> AgentCard {
    let mut schemes = BTreeMap::new();
    schemes.insert(
        "bearer".to_string(),
        SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
            scheme: "Bearer".into(),
            bearer_format: Some("JWT".into()),
            description: None,
        }),
    );
    schemes.insert(
        "key".to_string(),
        SecurityScheme::ApiKey(ApiKeySecurityScheme {
            location: "header".into(),
            name: "x-api-key".into(),
            description: None,
        }),
    );
    schemes.insert(
        "oauth".to_string(),
        SecurityScheme::OAuth2(Box::new(OAuth2SecurityScheme {
            flows: OAuthFlows::AuthorizationCode(AuthorizationCodeOAuthFlow {
                authorization_url: "https://a/authorize".into(),
                token_url: "https://a/token".into(),
                refresh_url: None,
                scopes: [("read".to_string(), "Read".to_string())].into(),
                pkce_required: Some(true),
            }),
            description: None,
            oauth2_metadata_url: None,
        })),
    );
    schemes.insert(
        "oidc".to_string(),
        SecurityScheme::OpenIdConnect(OpenIdConnectSecurityScheme {
            open_id_connect_url: "https://a/.well-known/openid-configuration".into(),
            description: None,
        }),
    );
    schemes.insert(
        "mtls".to_string(),
        SecurityScheme::MutualTls(MutualTlsSecurityScheme { description: None }),
    );
    let req = SecurityRequirement([("bearer".to_string(), vec![])].into());
    AgentCard {
        name: "maker".into(),
        description: "makes things".into(),
        version: "0.3.1".into(),
        supported_interfaces: vec![AgentInterface::jsonrpc(
            "http://localhost:8080/v1/agents/maker",
        )],
        capabilities: AgentCapabilities {
            streaming: Some(true),
            push_notifications: Some(false),
            extensions: vec![AgentExtension {
                uri: "https://distri.dev/a2a/extensions/event-stream/v1".into(),
                description: Some("distri events".into()),
                required: Some(false),
                params: Some(meta(&[("events", json!(["run_started"]))])),
            }],
            extended_agent_card: Some(false),
        },
        default_input_modes: vec!["text/plain".into()],
        default_output_modes: vec!["text/plain".into()],
        skills: vec![AgentSkill {
            id: "s".into(),
            name: "S".into(),
            description: "skill".into(),
            tags: vec!["t".into()],
            examples: vec!["e".into()],
            input_modes: Some(vec!["text/plain".into()]),
            output_modes: None,
            security_requirements: vec![req.clone()],
        }],
        provider: Some(AgentProvider {
            organization: "distri".into(),
            url: "https://distri.dev".into(),
        }),
        documentation_url: Some("https://distri.dev/docs".into()),
        icon_url: None,
        security_schemes: schemes,
        security_requirements: vec![req],
        signatures: vec![AgentCardSignature {
            protected: "eyJhbGciOiJFUzI1NiJ9".into(),
            signature: "c2ln".into(),
            header: None,
        }],
    }
}

#[test]
fn agent_card_conforms() {
    conforms("Agent Card", &card());
    let v = serde_json::to_value(card()).unwrap();
    assert_eq!(v["supportedInterfaces"][0]["protocolVersion"], "1.0");
    assert_eq!(v["version"], "0.3.1", "the agent's own version");
}

#[test]
fn every_part_kind_conforms() {
    for part in all_parts() {
        conforms("Part", &part);
    }
}

#[test]
fn message_conforms() {
    conforms("Message", &message());
}

#[test]
fn task_conforms_in_every_state() {
    for state in [
        TaskState::Submitted,
        TaskState::Working,
        TaskState::Completed,
        TaskState::Failed,
        TaskState::Canceled,
        TaskState::InputRequired,
        TaskState::Rejected,
        TaskState::AuthRequired,
    ] {
        conforms("Task", &task(state));
    }
}

#[test]
fn timestamps_are_iso_8601() {
    let v = serde_json::to_value(task(TaskState::Working)).unwrap();
    assert_eq!(v["status"]["timestamp"], "2026-11-20T12:00:00.000Z");
    // 0.x epoch-millisecond strings are still read.
    let s: TaskStatus = serde_json::from_value(json!({
        "state": "working", "timestamp": ts().timestamp_millis().to_string()
    }))
    .unwrap();
    assert_eq!(s.timestamp, Some(ts()));
}

#[test]
fn artifact_conforms() {
    conforms("Artifact", &artifact());
}

#[test]
fn stream_events_conform() {
    let status = StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
        task_id: "t1".into(),
        context_id: "c1".into(),
        status: TaskStatus {
            state: TaskState::Working,
            message: Some(message()),
            timestamp: Some(ts()),
        },
        metadata: Some(meta(&[("type", json!("run_started"))])),
    });
    let artifact_update = StreamResponse::ArtifactUpdate(TaskArtifactUpdateEvent {
        task_id: "t1".into(),
        context_id: "c1".into(),
        artifact: artifact(),
        append: Some(true),
        last_chunk: Some(false),
        metadata: None,
    });
    for ev in [
        status.clone(),
        artifact_update.clone(),
        StreamResponse::Task(task(TaskState::Completed)),
        StreamResponse::Message(message()),
    ] {
        conforms("Stream Response", &ev);
    }
    if let StreamResponse::StatusUpdate(s) = status {
        conforms("Task Status Update Event", &s);
    }
    if let StreamResponse::ArtifactUpdate(a) = artifact_update {
        conforms("Task Artifact Update Event", &a);
    }
}

#[test]
fn requests_and_responses_conform() {
    conforms(
        "Send Message Request",
        &SendMessageRequest {
            message: Message::new(Role::User, vec![Part::text("hi")]),
            configuration: Some(SendMessageConfiguration {
                accepted_output_modes: vec!["text/plain".into()],
                blocking: Some(true),
                history_length: Some(5),
                push_notification_config: None,
                return_immediately: None,
            }),
            metadata: Some(meta(&[("k", json!(1))])),
            tenant: None,
        },
    );
    conforms(
        "Send Message Response",
        &SendMessageResponse::Task(task(TaskState::Completed)),
    );
    conforms(
        "Send Message Response",
        &SendMessageResponse::Message(message()),
    );
    conforms(
        "List Tasks Response",
        &ListTasksResponse {
            tasks: vec![task(TaskState::Completed)],
            next_page_token: String::new(),
            page_size: 50,
            total_size: 1,
        },
    );
    conforms(
        "Get Task Request",
        &GetTaskRequest {
            id: "t1".into(),
            history_length: Some(3),
            tenant: None,
        },
    );
    conforms(
        "List Tasks Request",
        &ListTasksRequest {
            context_id: Some("c1".into()),
            status: Some(TaskState::Working),
            page_size: Some(10),
            ..Default::default()
        },
    );
}

#[test]
fn errors_use_a2a_codes() {
    let e = JsonRpcError::extension_support_required("urn:x");
    assert_eq!(e.code, -32008);
    let resp = JsonRpcResponse::error(Some(json!(1)), e);
    let v = serde_json::to_value(resp).unwrap();
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["error"]["code"], -32008);
    assert!(v.get("result").is_none());
    assert_eq!(JsonRpcError::task_not_found("x").code, -32001);
    assert_eq!(JsonRpcError::push_notification_not_supported().code, -32003);
}

#[test]
fn method_names_both_dialects() {
    use methods::{Dialect, Method};
    assert_eq!(
        Method::parse("SendStreamingMessage"),
        Some((Method::SendStreamingMessage, Dialect::V1))
    );
    assert_eq!(
        Method::parse("message/stream"),
        Some((Method::SendStreamingMessage, Dialect::Legacy))
    );
    assert_eq!(
        Method::parse("tasks/resubscribe"),
        Some((Method::SubscribeToTask, Dialect::Legacy))
    );
    assert_eq!(
        Method::parse("ListTasks"),
        Some((Method::ListTasks, Dialect::V1))
    );
    assert!(Method::parse("nope").is_none());
}

#[test]
fn legacy_input_is_still_read() {
    let m: Message = serde_json::from_value(json!({
        "kind": "message", "messageId": "m", "role": "user",
        "parts": [
            {"kind": "text", "text": "hi"},
            {"kind": "data", "data": {"part_type": "tool_result", "data": {}}},
            {"kind": "file", "file": {"uri": "https://x/a.png", "mimeType": "image/png", "name": "a.png"}}
        ]
    }))
    .unwrap();
    assert_eq!(m.role, Role::User);
    assert_eq!(
        m.parts[2].content,
        PartContent::Url("https://x/a.png".into())
    );
    assert_eq!(m.parts[2].media_type.as_deref(), Some("image/png"));
    assert_eq!(m.parts[2].filename.as_deref(), Some("a.png"));
    let ev: StreamResponse = serde_json::from_value(json!({
        "kind": "status-update", "taskId": "t", "contextId": "c", "final": true,
        "status": {"state": "input-required"}
    }))
    .unwrap();
    assert!(
        matches!(ev, StreamResponse::StatusUpdate(s) if s.status.state == TaskState::InputRequired)
    );
}

#[test]
fn legacy_downgrade_restores_0x_shapes() {
    let ev = StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
        task_id: "t".into(),
        context_id: "c".into(),
        status: TaskStatus {
            state: TaskState::Completed,
            message: None,
            timestamp: Some(ts()),
        },
        metadata: Some(meta(&[
            ("type", json!("run_finished")),
            ("final", json!(true)),
        ])),
    });
    let v = legacy::downgrade(serde_json::to_value(ev).unwrap());
    assert_eq!(v["kind"], "status-update");
    assert_eq!(v["final"], true);
    assert_eq!(v["status"]["state"], "completed");

    let v = legacy::downgrade(serde_json::to_value(StreamResponse::Message(message())).unwrap());
    assert_eq!(v["kind"], "message");
    assert_eq!(v["role"], "agent");
    assert_eq!(v["parts"][0], json!({"kind": "text", "text": "hello"}));
    assert_eq!(v["parts"][3]["kind"], "file");
    assert_eq!(v["parts"][3]["file"]["uri"], "https://example.com/a.pdf");
    assert_eq!(v["parts"][5]["data"]["part_type"], "tool_call");
    assert_eq!(v["parts"][5]["data"]["data"]["tool_call_id"], "c1");

    let v = legacy::downgrade(
        serde_json::to_value(SendMessageResponse::Task(task(TaskState::InputRequired))).unwrap(),
    );
    assert_eq!(v["kind"], "task");
    assert_eq!(
        v["status"]["state"], "inputRequired",
        "distri's 0.x spelling"
    );
}

#[test]
fn extensions_header_parsing() {
    assert_eq!(
        parse_extensions_header(["a, b", "b", " c "]),
        vec!["a".to_string(), "b".into(), "c".into()]
    );
}

/// The validator is not vacuous: 0.x shapes and stray fields fail.
#[test]
fn the_schema_rejects_0x_shapes() {
    let schema = |d: &str| jsonschema::validator_for(&schema_for(d)).unwrap();
    let mut card = serde_json::to_value(card()).unwrap();
    card["url"] = json!("http://x");
    assert!(
        !schema("Agent Card").is_valid(&card),
        "0.x card.url is not 1.0"
    );
    assert!(!schema("Part").is_valid(&json!({"kind": "text", "text": "x"})));
    assert!(!schema("Task Status").is_valid(&json!({"state": "working"})));
    assert!(!schema("Task Status Update Event").is_valid(&json!({"taskId": "t", "contextId": "c", "final": true, "status": {"state": "TASK_STATE_WORKING"}})));
}
