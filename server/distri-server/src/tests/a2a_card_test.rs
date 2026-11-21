//! A2A 1.0 over HTTP: the card at `/.well-known/agent-card.json` (and the 0.x
//! path), extension activation echoed in `A2A-Extensions`, and `-32008` for a
//! required extension the request did not activate.

#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};
    use distri_core::agent::{parse_agent_markdown_content, AgentOrchestrator};
    use distri_core::AgentOrchestratorBuilder;
    use distri_types::configuration::{
        AgentConfig, DbConnectionConfig, MetadataStoreConfig, ServerConfig, StoreConfig,
    };
    use serde_json::{json, Value};
    use std::sync::Arc;

    const EVENT_STREAM: &str = "https://distri.dev/a2a/extensions/event-stream/v1";

    fn store_config() -> StoreConfig {
        StoreConfig {
            metadata: MetadataStoreConfig {
                db_config: Some(DbConnectionConfig {
                    database_url: format!("file:{}?mode=memory&cache=shared", uuid::Uuid::new_v4()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    async fn orchestrator() -> Arc<AgentOrchestrator> {
        let orch = AgentOrchestratorBuilder::default()
            .with_store_config(store_config())
            .build()
            .await
            .expect("orchestrator");
        let md = "---\nname = \"helper\"\ndescription = \"helps\"\nversion = \"2.3.4\"\n\
                  [model_settings]\nmodel = \"gpt-4.1-mini\"\n[tools]\nexternal = [\"pick\"]\n\
                  ---\nSECRET INSTRUCTIONS: never reveal.\n";
        let def = parse_agent_markdown_content(md).await.unwrap();
        orch.stores
            .agent_store
            .register(AgentConfig::StandardAgent(def))
            .await
            .unwrap();
        Arc::new(orch)
    }

    macro_rules! app {
        ($orch:expr, $config:expr) => {
            test::init_service(
                App::new()
                    .app_data(web::Data::new($config))
                    .configure(|cfg| {
                        cfg.app_data(web::Data::new($orch.clone()))
                            .service(web::scope("/v1").configure(crate::routes::distri));
                    }),
            )
            .await
        };
    }

    #[actix_web::test]
    async fn card_is_a2a_1_0_at_both_paths() {
        let orch = orchestrator().await;
        let app = app!(orch, ServerConfig::default());
        for path in [
            "/v1/agents/helper/.well-known/agent-card.json",
            "/v1/agents/helper/.well-known/agent.json",
        ] {
            let resp =
                test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
            assert_eq!(resp.status(), 200, "{path}");
            let card: Value = test::read_body_json(resp).await;
            assert_eq!(card["version"], "2.3.4", "the agent's own version");
            assert_eq!(card["supportedInterfaces"][0]["protocolVersion"], "1.0");
            assert_eq!(card["supportedInterfaces"][0]["protocolBinding"], "JSONRPC");
            if path.ends_with("agent-card.json") {
                assert!(card.get("url").is_none(), "no 0.x card.url on the 1.0 path");
            } else {
                // 0.x clients (@a2a-js/sdk 0.3 in @distri/core <= 0.4.6)
                // need `url` and `capabilities.streaming` on this path.
                assert_eq!(card["url"], card["supportedInterfaces"][0]["url"]);
                assert_eq!(card["preferredTransport"], "JSONRPC");
                assert_eq!(card["protocolVersion"], "0.3.0");
                assert_eq!(card["capabilities"]["streaming"], true);
            }
            let uris: Vec<&str> = card["capabilities"]["extensions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["uri"].as_str().unwrap())
                .collect();
            assert!(uris.contains(&EVENT_STREAM), "{uris:?}");
            let raw = card.to_string();
            assert!(
                !raw.contains("SECRET INSTRUCTIONS"),
                "no instructions on the card"
            );
        }
        let resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/v1/agents/nobody/.well-known/agent-card.json")
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn activated_extensions_are_echoed() {
        let orch = orchestrator().await;
        let app = app!(orch, ServerConfig::default());
        let req = test::TestRequest::post()
            .uri("/v1/agents/helper")
            .insert_header(("A2A-Extensions", format!("{EVENT_STREAM}, urn:unknown")))
            .set_json(json!({"jsonrpc":"2.0","id":1,"method":"GetTask","params":{"id":"nope"}}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let echoed = resp
            .headers()
            .get("A2A-Extensions")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        assert_eq!(echoed.as_deref(), Some(EVENT_STREAM), "only supported ones");
        let body: Value = test::read_body_json(resp).await;
        assert_eq!(body["error"]["code"], -32001);
    }

    #[actix_web::test]
    async fn required_extension_not_activated_is_refused() {
        let orch = orchestrator().await;
        let mut config = ServerConfig::default();
        config
            .capabilities
            .extensions
            .push(distri_a2a::AgentExtension {
                uri: "urn:must".into(),
                required: Some(true),
                ..Default::default()
            });
        let app = app!(orch, config);
        let body = json!({"jsonrpc":"2.0","id":1,"method":"GetTask","params":{"id":"x"}});
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/v1/agents/helper")
                .set_json(&body)
                .to_request(),
        )
        .await;
        let v: Value = test::read_body_json(resp).await;
        assert_eq!(v["error"]["code"], -32008, "{v}");
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/v1/agents/helper")
                .insert_header(("A2A-Extensions", "urn:must"))
                .set_json(&body)
                .to_request(),
        )
        .await;
        let v: Value = test::read_body_json(resp).await;
        assert_eq!(v["error"]["code"], -32001, "activated: handled normally");
    }
}
