use tracing_subscriber::{filter::FilterFn, prelude::*, EnvFilter};

/// Initialize logging with sensible defaults for the agents library.
/// This will:
/// - Set up logging with the specified log level
/// - Filter out noisy logs from dependencies like hyper
/// - Format logs in a human-readable format
pub fn init_logging(level: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level))
        // Filter out noisy hyper logs
        .add_directive("hyper=off".parse().unwrap());

    // Only show our crate's logs and any errors from other crates
    let crate_filter = FilterFn::new(|metadata| {
        metadata.target().starts_with("agents") || metadata.level() <= &tracing::Level::ERROR
    });

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(crate_filter))
        .with(filter)
        .init();
}
