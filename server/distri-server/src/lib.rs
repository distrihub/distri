pub mod agent_server;
pub mod auth_routes;
pub mod context;
pub mod openapi;
pub mod routes;
pub mod routes_catalog;
pub mod server;

pub mod ui_server;

pub use routes_catalog::{distri_server_routes, DISTRI_SERVER_ROUTES};

#[cfg(test)]
mod stores;

#[cfg(test)]
mod tests;
