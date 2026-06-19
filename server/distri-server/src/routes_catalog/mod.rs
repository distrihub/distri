//! Route catalog — the auth-agnostic list of routes distri-server serves, with
//! a lightweight per-method [`Access`] weight.
//!
//! distri-server has no concept of authorization. Embedders (distri-cloud)
//! consume [`route_access`] to map each route's [`Access`] onto their own
//! authorization actions and assert their resolver agrees — so a route added
//! here (or a namespace the embedder doesn't classify) fails the embedder's
//! CI instead of silently shipping mis-scoped.
//!
//! Route paths, methods, and access weights are defined once in [`constants`]
//! via the `define_routes!` macro; `routes::distri` registers handlers using
//! [`Route::path`], never inline strings.

mod constants;

pub use constants::{route_access, Access, Route, DISTRI_SERVER_ROUTES};

/// `(path, &[(method, access)])` for every route distri-server registers.
/// Embedders prepend their mount scope (e.g. `/v1`) before matching.
pub fn distri_server_routes() -> &'static [(&'static str, &'static [(&'static str, Access)])] {
    DISTRI_SERVER_ROUTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_non_empty_and_well_formed() {
        let routes = distri_server_routes();
        assert!(
            routes.len() > 20,
            "expected >20 routes, got {}",
            routes.len()
        );
        for (path, methods) in routes {
            assert!(path.starts_with('/'), "path must start with /: {}", path);
            assert!(!methods.is_empty(), "no methods for {}", path);
            for (m, _access) in *methods {
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
    fn catalog_matches_route_enum() {
        // The macro generates both `Route::ALL` and `DISTRI_SERVER_ROUTES`
        // from the same input — assert they stay 1:1.
        assert_eq!(Route::ALL.len(), DISTRI_SERVER_ROUTES.len());
        for r in Route::ALL {
            assert!(
                DISTRI_SERVER_ROUTES
                    .iter()
                    .any(|(p, m)| *p == r.path() && *m == r.methods()),
                "Route::{:?} ({}) missing from DISTRI_SERVER_ROUTES",
                r,
                r.path()
            );
        }
    }

    #[test]
    fn a2a_dispatch_is_execute() {
        assert_eq!(Route::AgentDispatch.path(), "/agents/{id:.*}");
        let post = Route::AgentDispatch
            .methods()
            .iter()
            .find(|(m, _)| *m == "POST")
            .expect("POST on a2a dispatch");
        assert_eq!(post.1, Access::Execute);
    }

    #[test]
    fn agent_card_is_public() {
        let (_m, access) = Route::AgentCard.methods()[0];
        assert_eq!(access, Access::Public);
    }

    #[test]
    fn route_access_flattens_every_pair() {
        let flat: Vec<_> = route_access().collect();
        let expected: usize = DISTRI_SERVER_ROUTES.iter().map(|(_, m)| m.len()).sum();
        assert_eq!(flat.len(), expected);
        // Spot-check a known triple.
        assert!(flat
            .iter()
            .any(|(p, m, a)| *p == "/llm/execute" && *m == "POST" && *a == Access::Execute));
    }

    #[test]
    fn no_duplicate_paths() {
        let mut seen = std::collections::HashSet::new();
        for (path, _) in distri_server_routes() {
            assert!(seen.insert(*path), "duplicate path in catalog: {}", path);
        }
    }
}
