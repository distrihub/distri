use actix_web::web;

/// No-op: tool auth routes have been removed
pub fn configure_auth_routes(_cfg: &mut web::ServiceConfig) {}
