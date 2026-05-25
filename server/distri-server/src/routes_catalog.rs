//! Static catalog of routes registered by [`crate::routes::distri`].
//!
//! This is purely descriptive — no authorization concepts. distri-server is
//! auth-agnostic; embedders (distri-cloud) consume this catalog to assert
//! that every distri-server route has a corresponding authorization rule in
//! the embedder's own table. When you add a route in `routes.rs`, also add
//! it here, and the embedder's CI will tell you if you forgot a rule.
//!
//! The catalog uses leaf paths exactly as actix registers them (relative —
//! no `/v1` prefix). Embedders prepend their mount scope before lookup.

/// `(path_pattern, &[supported_methods])` — one entry per `.service(...)`
/// call in `routes::distri`.
pub const DISTRI_SERVER_ROUTES: &[(&str, &[&str])] = &[
    // Agent card (a2a spec — intentionally public)
    ("/agents/{name}/.well-known/agent.json", &["GET"]),
    // Agent CRUD + a2a entry
    ("/agents", &["GET", "POST"]),
    ("/agents/{id:.*}/validate", &["GET"]),
    ("/agents/{id:.*}/complete-tool", &["POST"]),
    ("/agents/{id:.*}/dag", &["GET"]),
    ("/agents/{id:.*}", &["GET", "POST", "PUT", "DELETE"]),
    // Hooks
    ("/event/hooks", &["POST"]),
    // Tasks / threads / tools / messages
    ("/tasks", &["GET"]),
    ("/tools", &["GET"]),
    ("/threads", &["GET"]),
    ("/threads/agents", &["GET"]),
    ("/threads/{thread_id}/messages", &["GET"]),
    ("/threads/{thread_id}", &["GET", "PUT", "DELETE"]),
    (
        "/threads/{thread_id}/messages/{message_id}/read",
        &["GET", "POST"],
    ),
    ("/threads/{thread_id}/read-status", &["GET"]),
    (
        "/threads/{thread_id}/messages/{message_id}/vote",
        &["GET", "POST", "DELETE"],
    ),
    (
        "/threads/{thread_id}/messages/{message_id}/votes",
        &["GET"],
    ),
    // Schema + meta
    ("/schema/agent", &["GET"]),
    ("/device", &["GET"]),
    ("/home/stats", &["GET"]),
    // Files / sessions / artifacts (paths under each scope are wildcards
    // because submodules expand them; embedders should rule on the prefix)
    ("/files/*", &["GET", "POST", "PUT", "DELETE"]),
    ("/sessions/*", &["GET", "POST", "PUT", "DELETE"]),
    ("/artifacts/*", &["GET", "POST", "PUT", "DELETE"]),
    // Build / browser / llm / proxy
    ("/build", &["POST"]),
    ("/browser/session", &["POST"]),
    ("/llm/execute", &["POST"]),
    ("/request", &["POST"]),
];

/// Convenience accessor.
pub fn distri_server_routes() -> &'static [(&'static str, &'static [&'static str])] {
    DISTRI_SERVER_ROUTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_non_empty_and_well_formed() {
        let routes = distri_server_routes();
        assert!(routes.len() > 20, "expected >20 routes, got {}", routes.len());
        for (path, methods) in routes {
            assert!(path.starts_with('/'), "path must start with /: {}", path);
            assert!(!methods.is_empty(), "no methods for {}", path);
            for m in *methods {
                assert!(
                    matches!(
                        *m,
                        "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD" | "OPTIONS"
                    ),
                    "unknown method `{}` on {}",
                    m,
                    path
                );
            }
        }
    }

    #[test]
    fn a2a_dispatch_path_is_listed() {
        let routes = distri_server_routes();
        let has_a2a = routes
            .iter()
            .any(|(p, m)| *p == "/agents/{id:.*}" && m.contains(&"POST"));
        assert!(has_a2a, "POST /agents/{{id:.*}} must be in the catalog");
    }

    #[test]
    fn no_duplicate_paths() {
        let routes = distri_server_routes();
        let mut seen = std::collections::HashSet::new();
        for (path, _) in routes {
            assert!(seen.insert(*path), "duplicate path in catalog: {}", path);
        }
    }
}
