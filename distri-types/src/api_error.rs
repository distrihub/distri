//! `ApiError` — the single error type that every distri service returns.
//!
//! Variants map cleanly to HTTP status codes. Routes return
//! `Result<HttpResponse, ApiError>`; the `ResponseError` impl lives in
//! `distri-server` (where the actix dependency lives) and renders every
//! variant as `{"error": "<message>"}` JSON with the appropriate status.
//!
//! Store calls return `anyhow::Result<T>`; the `#[from] anyhow::Error`
//! conversion lets services `?` straight through, surfacing unexpected
//! errors as `ApiError::Internal` (logged + 500). Business decisions
//! (validation failures, "not found", "this is forbidden") explicitly
//! return the typed variant — no string-parsing at the boundary.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    /// Caller's input is malformed or violates a documented rule.
    /// Maps to HTTP 400.
    #[error("{0}")]
    BadRequest(String),

    /// No authenticated session, or the session is invalid/expired.
    /// Maps to HTTP 401.
    #[error("{0}")]
    Unauthorized(String),

    /// Authenticated, but the operation is not permitted for this caller
    /// (e.g. mutating an `is_system=true` row). Maps to HTTP 403.
    #[error("{0}")]
    Forbidden(String),

    /// Entity does not exist. Maps to HTTP 404.
    #[error("{0}")]
    NotFound(String),

    /// Operation would violate a uniqueness constraint or a state
    /// invariant (e.g. duplicate name in workspace). Maps to HTTP 409.
    #[error("{0}")]
    Conflict(String),

    /// Request shape is valid but its content fails domain validation
    /// (e.g. a referenced credential's material is wrong for this flow).
    /// Maps to HTTP 422.
    #[error("{0}")]
    Unprocessable(String),

    /// Backing service unavailable (store not wired, OAuth not configured).
    /// Maps to HTTP 503.
    #[error("{0}")]
    ServiceUnavailable(String),

    /// Wraps an unexpected error (DB, IO, serde, anything else). Logged at
    /// the route boundary; surfaced as a generic HTTP 500 to the client so
    /// internal details don't leak.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl ApiError {
    /// HTTP status this variant maps to.
    pub fn status(&self) -> u16 {
        match self {
            Self::BadRequest(_) => 400,
            Self::Unauthorized(_) => 401,
            Self::Forbidden(_) => 403,
            Self::NotFound(_) => 404,
            Self::Conflict(_) => 409,
            Self::Unprocessable(_) => 422,
            Self::ServiceUnavailable(_) => 503,
            Self::Internal(_) => 500,
        }
    }

    /// Client-safe message. `Internal` returns a generic string — the
    /// actual error is logged server-side via the `ResponseError` impl
    /// instead of being leaked to the client.
    pub fn message(&self) -> String {
        match self {
            Self::Internal(_) => "internal server error".to_string(),
            other => other.to_string(),
        }
    }
}

// ── Constructors — terse call sites: `ApiError::not_found("...")` etc.
impl ApiError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::Unauthorized(msg.into())
    }
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::Forbidden(msg.into())
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Conflict(msg.into())
    }
    pub fn unprocessable(msg: impl Into<String>) -> Self {
        Self::Unprocessable(msg.into())
    }
    pub fn service_unavailable(msg: impl Into<String>) -> Self {
        Self::ServiceUnavailable(msg.into())
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

// ── Actix integration (feature = "actix") ───────────────────────────────
//
// Putting the impl here (vs. in distri-server) sidesteps Rust's orphan
// rule: `ResponseError` and `ApiError` need to be in the same crate. Off
// by default; distri-server / distri-cloud opt in via the `actix` feature.
#[cfg(feature = "actix")]
impl actix_web::ResponseError for ApiError {
    fn status_code(&self) -> actix_web::http::StatusCode {
        actix_web::http::StatusCode::from_u16(self.status())
            .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR)
    }

    fn error_response(&self) -> actix_web::HttpResponse {
        if let ApiError::Internal(e) = self {
            tracing::error!("internal error: {:#}", e);
        }
        actix_web::HttpResponse::build(self.status_code())
            .json(serde_json::json!({ "error": self.message() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes() {
        assert_eq!(ApiError::bad_request("x").status(), 400);
        assert_eq!(ApiError::unauthorized("x").status(), 401);
        assert_eq!(ApiError::forbidden("x").status(), 403);
        assert_eq!(ApiError::not_found("x").status(), 404);
        assert_eq!(ApiError::conflict("x").status(), 409);
        assert_eq!(ApiError::unprocessable("x").status(), 422);
        assert_eq!(ApiError::service_unavailable("x").status(), 503);
        assert_eq!(ApiError::Internal(anyhow::anyhow!("oops")).status(), 500);
    }

    #[test]
    fn internal_message_is_generic() {
        let e = ApiError::Internal(anyhow::anyhow!("db failed: ..."));
        assert_eq!(e.message(), "internal server error");
    }

    #[test]
    fn anyhow_conversion_is_internal() {
        let e: ApiError = anyhow::anyhow!("any error").into();
        assert!(matches!(e, ApiError::Internal(_)));
    }
}
