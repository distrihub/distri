//! `HttpDecisionProvider` against recorded System One responses.
//!
//! `fixtures/kev_llama_server_response.json` was recorded from stock
//! llama.cpp `llama-server` (b11361+, CPU) serving `ggml-org/Kev-0.8B-GGUF`
//! Q8_0, answering `fixtures/kev_request.json`.
//!
//! `live_llama_server_conformance` runs the same request against a real
//! server when `DISTRI_DECISION_LIVE_URL` is set (e.g.
//! `https://decisions-dev.distri.dev/v1`, key in `DISTRI_DECISION_LIVE_KEY`);
//! without it there is nothing to talk to and the test returns early.

use distri_types::decision::{
    systemone_url, DecisionAnswer, DecisionError, DecisionRequest, DecisionWire,
};
use llm_gateway::{DecisionEndpoint, DecisionProvider, HttpDecisionProvider};
use wiremock::matchers::{bearer_token, body_partial_json, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const REQUEST: &str = include_str!("fixtures/kev_request.json");
const LLAMA_SERVER_RESPONSE: &str = include_str!("fixtures/kev_llama_server_response.json");

fn request(model: &str) -> DecisionRequest {
    let mut req: DecisionRequest = serde_json::from_str(REQUEST).unwrap();
    req.model = model.to_string();
    req
}

fn provider(url: String, api_key: Option<&str>, wire: DecisionWire) -> HttpDecisionProvider {
    HttpDecisionProvider::new(
        reqwest::Client::new(),
        DecisionEndpoint {
            url,
            api_key: api_key.map(str::to_string),
            wire,
        },
    )
}

fn assert_distribution(probabilities: impl Iterator<Item = f64>) {
    let sum: f64 = probabilities.sum();
    assert!((sum - 1.0).abs() < 1e-3, "probabilities sum to {sum}");
}

#[tokio::test]
async fn systemone_round_trip_with_recorded_llama_server_answer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(bearer_token("sk-test"))
        // Distri's "provider/model" prefix is stripped before it goes upstream.
        .and(body_partial_json(serde_json::json!({"model": "kev-0.8b"})))
        .respond_with(ResponseTemplate::new(200).set_body_string(LLAMA_SERVER_RESPONSE))
        .expect(1)
        .mount(&server)
        .await;

    let res = provider(
        systemone_url(&format!("{}/v1", server.uri())),
        Some("sk-test"),
        DecisionWire::Systemone,
    )
    .decide(&request("my_llama/kev-0.8b"))
    .await
    .unwrap();

    assert_eq!(res.usage.input_tokens, 173);
    assert_eq!(res.usage.output_tokens, 0);
    assert_eq!(
        res.answers.keys().collect::<Vec<_>>(),
        ["route", "needs_approval", "urgency"]
    );
    let DecisionAnswer::Choice {
        choice,
        probabilities,
        confidence,
    } = &res.answers["route"]
    else {
        panic!("route should be a choice");
    };
    assert_eq!(choice, "finance_ops");
    assert!(*confidence > 0.5);
    assert_distribution(probabilities.values().copied());
    assert!(
        matches!(res.answers["needs_approval"], DecisionAnswer::Noul { noul } if (0.0..=1.0).contains(&noul))
    );
    let DecisionAnswer::Score {
        legend,
        probabilities,
        ..
    } = &res.answers["urgency"]
    else {
        panic!("urgency should be a score");
    };
    assert_eq!(legend["0"], "not urgent");
    assert_distribution(probabilities.values().copied());
}

#[tokio::test]
async fn no_authorization_header_without_a_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(LLAMA_SERVER_RESPONSE))
        .mount(&server)
        .await;

    provider(server.uri(), None, DecisionWire::Systemone)
        .decide(&request("my_llama/kev"))
        .await
        .unwrap();

    let received: Vec<Request> = server.received_requests().await.unwrap();
    assert!(!received[0].headers.contains_key("authorization"));
}

#[tokio::test]
async fn cloudflare_envelope_is_unwrapped() {
    let server = MockServer::start().await;
    let inner: serde_json::Value = serde_json::from_str(LLAMA_SERVER_RESPONSE).unwrap();
    Mock::given(method("POST"))
        .and(path(
            "/client/v4/accounts/acc/ai/run/@cf/cloudflare/clef-flash",
        ))
        .and(body_partial_json(
            serde_json::json!({"model": "clef-flash"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": inner, "success": true, "errors": [], "messages": []
        })))
        .mount(&server)
        .await;

    let res = provider(
        format!(
            "{}/client/v4/accounts/acc/ai/run/@cf/cloudflare/clef-flash",
            server.uri()
        ),
        Some("cf-token"),
        DecisionWire::CloudflareEnvelope,
    )
    .decide(&request("workers_ai/clef-flash"))
    .await
    .unwrap();
    assert!(
        matches!(&res.answers["route"], DecisionAnswer::Choice { choice, .. } if choice == "finance_ops")
    );
}

#[tokio::test]
async fn cloudflare_failure_envelope_surfaces_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "result": null, "success": false,
            "errors": [{"code": 5006, "message": "questions: too many options"}], "messages": []
        })))
        .mount(&server)
        .await;

    let err = provider(server.uri(), Some("t"), DecisionWire::CloudflareEnvelope)
        .decide(&request("workers_ai/clef-flash"))
        .await
        .unwrap_err();
    match err {
        DecisionError::Provider { body, .. } => {
            assert!(body.contains("5006: questions: too many options"))
        }
        other => panic!("expected Provider error, got {other:?}"),
    }
}

#[tokio::test]
async fn provider_errors_keep_status_and_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(422)
                .set_body_string(r#"{"error":"choice has too many options"}"#),
        )
        .mount(&server)
        .await;

    let err = provider(server.uri(), None, DecisionWire::Systemone)
        .decide(&request("my_llama/kev"))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, DecisionError::Provider { status: 422, body } if body.contains("too many options"))
    );
    assert_eq!(distri_types::ApiError::from(err).status(), 422);
}

#[tokio::test]
async fn unreadable_body_is_a_decode_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>gateway</html>"))
        .mount(&server)
        .await;

    let err = provider(server.uri(), None, DecisionWire::Systemone)
        .decide(&request("my_llama/kev"))
        .await
        .unwrap_err();
    assert!(matches!(err, DecisionError::Decode(_)));
    assert_eq!(distri_types::ApiError::from(err).status(), 502);
}

#[tokio::test]
async fn invalid_requests_never_reach_the_network() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(LLAMA_SERVER_RESPONSE))
        .expect(0)
        .mount(&server)
        .await;

    let p = provider(server.uri(), None, DecisionWire::Systemone);
    let mut req = request("my_llama/kev");
    req.questions.clear();
    assert!(matches!(
        p.decide(&req).await,
        Err(DecisionError::InvalidRequest(_))
    ));
    assert!(matches!(
        p.decide(&request("no-provider-prefix")).await,
        Err(DecisionError::InvalidModel(_))
    ));
}

#[tokio::test]
async fn live_llama_server_conformance() {
    let Ok(base_url) = std::env::var("DISTRI_DECISION_LIVE_URL") else {
        eprintln!("DISTRI_DECISION_LIVE_URL not set; skipping live llama-server conformance");
        return;
    };
    let key = std::env::var("DISTRI_DECISION_LIVE_KEY").ok();
    let res = provider(
        systemone_url(&base_url),
        key.as_deref(),
        DecisionWire::Systemone,
    )
    .decide(&request("local/kev"))
    .await
    .unwrap();

    assert_eq!(res.usage.output_tokens, 0);
    assert!(res.usage.input_tokens > 0);
    // Same question, same model: the recorded answer and the live one agree
    // within float noise (llama.cpp is not bit-deterministic across batches).
    let recorded: serde_json::Value = serde_json::from_str(LLAMA_SERVER_RESPONSE).unwrap();
    let DecisionAnswer::Choice {
        choice,
        probabilities,
        ..
    } = &res.answers["route"]
    else {
        panic!("route should be a choice");
    };
    assert_eq!(
        choice,
        recorded["answers"]["route"]["choice"].as_str().unwrap()
    );
    for (option, p) in probabilities {
        let want = recorded["answers"]["route"]["probabilities"][option]
            .as_f64()
            .unwrap();
        assert!(
            (p - want).abs() < 0.02,
            "{option}: live {p} vs recorded {want}"
        );
    }
    let DecisionAnswer::Score { probabilities, .. } = &res.answers["urgency"] else {
        panic!("urgency should be a score");
    };
    assert_distribution(probabilities.values().copied());
}
