//! Integration tests for optional deployment-token auth.
//!
//! Two credentials, not interchangeable: the deployment secret mints, an
//! access token passes the API surface. Off is the default and means no
//! check runs at all.

#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};
    use distri_auth::TokenAuth;
    use serde_json::Value;

    const SECRET: &str = "deployment-secret-long-enough";

    fn auth() -> TokenAuth {
        TokenAuth::new(SECRET, 3600, 86_400).expect("valid secret")
    }

    /// The /v1 surface with auth on: guard wrapped, TokenAuth registered.
    macro_rules! guarded_app {
        () => {
            test::init_service(
                App::new().app_data(web::Data::new(auth())).service(
                    web::scope("/v1")
                        .configure(|cfg| {
                            cfg.service(
                                web::resource("/token")
                                    .route(web::post().to(crate::token_auth::issue_token)),
                            )
                            .service(
                                web::resource("/probe").route(web::get().to(|| async {
                                    actix_web::HttpResponse::Ok().body("reached")
                                })),
                            );
                        })
                        .wrap(actix_web::middleware::from_fn(
                            crate::token_auth::require_access_token,
                        )),
                ),
            )
            .await
        };
    }

    /// The same surface with auth off: no guard, no TokenAuth.
    macro_rules! open_app {
        () => {
            test::init_service(App::new().service(web::scope("/v1").configure(|cfg| {
                cfg.service(
                    web::resource("/token").route(web::post().to(crate::token_auth::issue_token)),
                )
                .service(web::resource("/probe").route(
                    web::get().to(|| async { actix_web::HttpResponse::Ok().body("reached") }),
                ));
            })))
            .await
        };
    }

    /// Mint a pair with the deployment secret and return the JSON body.
    macro_rules! mint {
        ($app:expr) => {{
            let req = test::TestRequest::post()
                .uri("/v1/token")
                .insert_header(("x-api-key", SECRET))
                .to_request();
            let resp = test::call_service(&$app, req).await;
            assert_eq!(resp.status(), 200, "minting with the secret should succeed");
            let body: Value = test::read_body_json(resp).await;
            body
        }};
    }

    // ── minting ───────────────────────────────────────────────────────────

    #[actix_web::test]
    async fn minting_requires_the_deployment_secret() {
        let app = guarded_app!();

        let resp = test::call_service(
            &app,
            test::TestRequest::post().uri("/v1/token").to_request(),
        )
        .await;
        assert_eq!(resp.status(), 401, "no credential must not mint");

        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/v1/token")
                .insert_header(("x-api-key", "not-the-secret"))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 401, "a wrong secret must not mint");
    }

    #[actix_web::test]
    async fn minting_returns_the_shared_token_response_contract() {
        let app = guarded_app!();
        let body = mint!(app);

        // The exact shape `Distri::issue_token()` deserializes.
        assert!(body["access_token"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(body["refresh_token"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert!(body["expires_at"].as_i64().is_some());
    }

    #[actix_web::test]
    async fn the_deployment_secret_is_never_in_the_response() {
        let app = guarded_app!();
        let body = mint!(app);
        let rendered = body.to_string();
        assert!(
            !rendered.contains(SECRET),
            "the deployment secret must never reach a browser: {rendered}"
        );
    }

    #[actix_web::test]
    async fn a_bearer_deployment_secret_also_mints() {
        let app = guarded_app!();
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/v1/token")
                .insert_header(("Authorization", format!("Bearer {SECRET}")))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 200);
    }

    // ── verification ──────────────────────────────────────────────────────

    #[actix_web::test]
    async fn a_minted_access_token_passes_the_guard() {
        let app = guarded_app!();
        let token = mint!(app)["access_token"].as_str().unwrap().to_string();

        let resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/v1/probe")
                .insert_header(("Authorization", format!("Bearer {token}")))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 200);
    }

    #[actix_web::test]
    async fn anything_that_is_not_an_access_token_is_401() {
        let app = guarded_app!();
        let minted = mint!(app);

        let cases: Vec<(&str, Option<String>)> = vec![
            ("no header at all", None),
            ("garbage", Some("Bearer not-a-token".to_string())),
            ("not a bearer", Some(format!("ApiKey {SECRET}"))),
            // The deployment secret mints; it does not pass the API surface.
            ("the deployment secret", Some(format!("Bearer {SECRET}"))),
            // A refresh token renews; it is not an access token.
            (
                "a refresh token",
                Some(format!(
                    "Bearer {}",
                    minted["refresh_token"].as_str().unwrap()
                )),
            ),
        ];

        for (label, header) in cases {
            let mut req = test::TestRequest::get().uri("/v1/probe");
            if let Some(value) = header {
                req = req.insert_header(("Authorization", value));
            }
            let resp = test::call_service(&app, req.to_request()).await;
            assert_eq!(resp.status(), 401, "{label} must be refused");
        }
    }

    #[actix_web::test]
    async fn an_expired_access_token_is_401() {
        // A one-second access token, already past its expiry.
        let auth = TokenAuth::new(SECRET, 1, 86_400).unwrap();
        let stale = auth.mint_pair_at(0).access_token;

        let app = test::init_service(
            App::new().app_data(web::Data::new(auth)).service(
                web::scope("/v1")
                    .configure(|cfg| {
                        cfg.service(
                            web::resource("/probe")
                                .route(web::get().to(|| async {
                                    actix_web::HttpResponse::Ok().body("reached")
                                })),
                        );
                    })
                    .wrap(actix_web::middleware::from_fn(
                        crate::token_auth::require_access_token,
                    )),
            ),
        )
        .await;

        let resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/v1/probe")
                .insert_header(("Authorization", format!("Bearer {stale}")))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 401);
    }

    // ── refresh ───────────────────────────────────────────────────────────

    #[actix_web::test]
    async fn a_refresh_token_renews_without_the_deployment_secret() {
        let app = guarded_app!();
        let first = mint!(app);

        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/v1/token")
                .set_json(serde_json::json!({
                    "grant_type": "refresh_token",
                    "refresh_token": first["refresh_token"],
                }))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 200, "refresh needs no deployment secret");

        let second: Value = test::read_body_json(resp).await;
        let renewed = second["access_token"].as_str().unwrap();
        assert_ne!(renewed, first["access_token"].as_str().unwrap());

        let resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/v1/probe")
                .insert_header(("Authorization", format!("Bearer {renewed}")))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 200, "the renewed token works");
    }

    #[actix_web::test]
    async fn a_bogus_refresh_token_renews_nothing() {
        let app = guarded_app!();
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/v1/token")
                .set_json(serde_json::json!({ "refresh_token": "nope" }))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 401);
    }

    // ── off ───────────────────────────────────────────────────────────────

    #[actix_web::test]
    async fn with_auth_off_the_api_needs_no_token() {
        let app = open_app!();
        let resp =
            test::call_service(&app, test::TestRequest::get().uri("/v1/probe").to_request()).await;
        assert_eq!(resp.status(), 200, "zero-config local runs must not break");
    }

    #[actix_web::test]
    async fn with_auth_off_the_mint_endpoint_does_not_issue_tokens() {
        let app = open_app!();
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/v1/token")
                .insert_header(("x-api-key", SECRET))
                .to_request(),
        )
        .await;
        assert_eq!(
            resp.status(),
            404,
            "no secret is configured, so there is nothing to mint against"
        );
    }
}
