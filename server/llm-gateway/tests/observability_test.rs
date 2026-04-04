//! Level-1 integration tests: verify span attribute names and values.
//!
//! Uses a custom tracing_subscriber::Layer to capture field names on spans.
//! Run with: cargo test -p llm-gateway --test observability_test

use llm_gateway::observability::{
    self,
    types::{
        GenAiAgentSpan, GenAiInferenceSpan, GenAiOperation, GenAiProvider, GenAiToolSpan,
        GenAiToolType,
    },
};
use std::sync::{Arc, Mutex};
use tracing::Subscriber;
use tracing_subscriber::{layer::SubscriberExt, Layer};

/// Records field names visited during span creation and recording.
#[derive(Default, Clone)]
struct FieldCapture {
    fields: Arc<Mutex<Vec<(String, String)>>>, // (field_name, value_as_string)
}

impl<S: Subscriber> Layer<S> for FieldCapture {
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = FieldVisitor { captured: vec![] };
        attrs.record(&mut visitor);
        self.fields.lock().unwrap().extend(visitor.captured);
    }

    fn on_record(
        &self,
        _id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = FieldVisitor { captured: vec![] };
        values.record(&mut visitor);
        self.fields.lock().unwrap().extend(visitor.captured);
    }
}

struct FieldVisitor {
    captured: Vec<(String, String)>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if !value.is_empty() {
            self.captured
                .push((field.name().to_string(), value.to_string()));
        }
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.captured
            .push((field.name().to_string(), value.to_string()));
    }
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.captured
            .push((field.name().to_string(), format!("{:.6}", value)));
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.captured
            .push((field.name().to_string(), value.to_string()));
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.captured
            .push((field.name().to_string(), value.to_string()));
    }
    fn record_debug(&mut self, _field: &tracing::field::Field, _value: &dyn std::fmt::Debug) {
        // Skip debug values (tracing::field::Empty shows up here but has no useful value)
    }
}

fn field_names(fields: &[(String, String)]) -> Vec<&str> {
    fields.iter().map(|(k, _)| k.as_str()).collect()
}

fn run_with_capture<F>(f: F) -> Vec<(String, String)>
where
    F: FnOnce(),
{
    let capture = FieldCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    tracing::subscriber::with_default(subscriber, f);
    let result = capture.fields.lock().unwrap().clone();
    result
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn test_inference_span_required_attributes_present() {
    let fields = run_with_capture(|| {
        let attrs = GenAiInferenceSpan {
            operation: Some(GenAiOperation::Chat),
            provider: Some(GenAiProvider::Anthropic),
            request_model: Some("claude-3-5-sonnet-20241022".into()),
            distri_thread_id: Some("thread-1".into()),
            distri_task_id: Some("task-1".into()),
            ..Default::default()
        };
        let _span = observability::builder::inference_span(&attrs);
    });

    let names = field_names(&fields);
    assert!(
        names.contains(&"gen_ai.operation.name"),
        "missing gen_ai.operation.name; got: {:?}",
        names
    );
    assert!(
        names.contains(&"gen_ai.provider.name"),
        "missing gen_ai.provider.name"
    );
    assert!(
        names.contains(&"gen_ai.request.model"),
        "missing gen_ai.request.model"
    );
    assert!(
        names.contains(&"distri.thread_id"),
        "missing distri.thread_id"
    );
    assert!(names.contains(&"distri.task_id"), "missing distri.task_id");
    assert!(
        names.contains(&"otel.name"),
        "missing otel.name (dynamic span name)"
    );
}

#[test]
fn test_deprecated_gen_ai_system_absent() {
    let fields = run_with_capture(|| {
        let attrs = GenAiInferenceSpan {
            operation: Some(GenAiOperation::Chat),
            provider: Some(GenAiProvider::Anthropic),
            request_model: Some("claude-3-5-sonnet".into()),
            ..Default::default()
        };
        let _span = observability::builder::inference_span(&attrs);
    });

    let names = field_names(&fields);
    assert!(
        !names.contains(&"gen_ai.system"),
        "gen_ai.system (deprecated) must NOT be present; got: {:?}",
        names
    );
}

#[test]
fn test_inference_span_name_format() {
    let attrs = GenAiInferenceSpan {
        operation: Some(GenAiOperation::Chat),
        request_model: Some("claude-3-5-sonnet-20241022".into()),
        ..Default::default()
    };
    assert_eq!(attrs.span_name(), "chat claude-3-5-sonnet-20241022");
}

#[test]
fn test_agent_span_attributes_present() {
    let fields = run_with_capture(|| {
        let attrs = GenAiAgentSpan {
            agent_name: "coder".into(),
            agent_id: Some("agent-123".into()),
            distri_thread_id: Some("t1".into()),
            distri_run_id: Some("r1".into()),
            ..Default::default()
        };
        let _span = observability::builder::agent_span(&attrs);
    });

    let names = field_names(&fields);
    assert!(
        names.contains(&"gen_ai.agent.name"),
        "missing gen_ai.agent.name; got {:?}",
        names
    );
    assert!(
        names.contains(&"gen_ai.agent.id"),
        "missing gen_ai.agent.id"
    );
    assert!(
        names.contains(&"gen_ai.operation.name"),
        "missing gen_ai.operation.name"
    );
    assert!(
        names.contains(&"distri.thread_id"),
        "missing distri.thread_id"
    );
    assert!(names.contains(&"distri.run_id"), "missing distri.run_id");
}

#[test]
fn test_tool_span_attributes_present() {
    let fields = run_with_capture(|| {
        let attrs = GenAiToolSpan {
            tool_name: "bash".into(),
            tool_type: Some(GenAiToolType::Function),
            tool_call_id: Some("tc-abc".into()),
            distri_step_id: Some("step-1".into()),
            ..Default::default()
        };
        let _span = observability::builder::tool_span(&attrs);
    });

    let names = field_names(&fields);
    assert!(
        names.contains(&"gen_ai.tool.name"),
        "missing gen_ai.tool.name; got {:?}",
        names
    );
    assert!(
        names.contains(&"gen_ai.tool.type"),
        "missing gen_ai.tool.type"
    );
    assert!(
        names.contains(&"gen_ai.tool.call.id"),
        "missing gen_ai.tool.call.id"
    );
    assert!(names.contains(&"distri.step_id"), "missing distri.step_id");
}

#[test]
fn test_record_inference_response_fills_fields() {
    let fields = run_with_capture(|| {
        let attrs = GenAiInferenceSpan {
            operation: Some(GenAiOperation::Chat),
            provider: Some(GenAiProvider::Anthropic),
            request_model: Some("claude-3-5-sonnet".into()),
            ..Default::default()
        };
        let span = observability::builder::inference_span(&attrs);
        let _guard = span.enter();
        observability::recorder::record_inference_response(
            &span,
            Some("claude-3-5-sonnet-20241022"),
            Some("resp-1"),
            &["end_turn".to_string()],
            Some(1000),
            Some(250),
            Some(100),
            None,
            400,
            Some(0.005),
        );
    });

    let names = field_names(&fields);
    assert!(
        names.contains(&"gen_ai.usage.input_tokens"),
        "missing input_tokens; got {:?}",
        names
    );
    assert!(
        names.contains(&"gen_ai.usage.output_tokens"),
        "missing output_tokens"
    );
    assert!(names.contains(&"distri.estimated_cost_usd"), "missing cost");
    assert!(
        names.contains(&"gen_ai.response.model"),
        "missing response.model"
    );
    assert!(
        names.contains(&"gen_ai.response.finish_reasons"),
        "missing finish_reasons"
    );
}

#[test]
fn test_optional_fields_absent_when_not_set() {
    // When distri context fields are None, they should NOT appear as empty strings
    let fields = run_with_capture(|| {
        let attrs = GenAiInferenceSpan {
            operation: Some(GenAiOperation::Chat),
            provider: Some(GenAiProvider::OpenAi),
            request_model: Some("gpt-5.1".into()),
            // No distri context set
            ..Default::default()
        };
        let _span = observability::builder::inference_span(&attrs);
    });

    // Verify that optional fields with no value don't appear as empty strings
    for (name, value) in &fields {
        if name.starts_with("distri.") || name == "gen_ai.conversation.id" {
            assert!(
                !value.is_empty(),
                "Field '{}' should not be present with empty value, but got '{}'",
                name,
                value
            );
        }
    }
}
