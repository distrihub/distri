//! Route catalog — the auth-agnostic list of routes distri-server serves.
//!
//! distri-server has no concept of authorization. Embedders (distri-cloud)
//! consume [`distri_server_routes`] to assert every route has a matching
//! authorization rule in their own table, so coverage can't silently drift.
//!
//! Route paths + methods are defined once in [`constants`] via the
//! `define_routes!` macro; `routes::distri` registers handlers using
//! [`Route::path`], never inline strings.

mod constants;

pub use constants::{Route, DISTRI_SERVER_ROUTES};

/// `(path, methods)` for every route distri-server registers. Embedders
/// prepend their mount scope (e.g. `/v1`) before matching against their
/// own authorization table.
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
    fn a2a_dispatch_path_is_listed() {
        assert_eq!(Route::AgentDispatch.path(), "/agents/{id:.*}");
        assert!(Route::AgentDispatch.methods().contains(&"POST"));
    }

    #[test]
    fn no_duplicate_paths() {
        let mut seen = std::collections::HashSet::new();
        for (path, _) in distri_server_routes() {
            assert!(seen.insert(*path), "duplicate path in catalog: {}", path);
        }
    }
}
