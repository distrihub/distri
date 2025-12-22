use tracing_subscriber::{filter::FilterFn, fmt::format::FmtSpan, prelude::*, EnvFilter};

/// Initialize logging with sensible defaults for the agents library.
pub fn init_logging(level: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level))
        // Filter out noisy hyper logs
        .add_directive("hyper=off".parse().unwrap())
        .add_directive("rustyline=off".parse().unwrap())
        .add_directive("h2=off".parse().unwrap())
        .add_directive("rustls=off".parse().unwrap())
        .add_directive("swc_common=off".parse().unwrap())
        .add_directive("swc_ecma_transforms_base=off".parse().unwrap())
        .add_directive("reqwest=off".parse().unwrap())
        .add_directive("async_mcp=off".parse().unwrap())
        .add_directive("mcp_crawl=off".parse().unwrap())
        .add_directive("html5ever=off".parse().unwrap())
        .add_directive("selectors=off".parse().unwrap())
        .add_directive("handlebars=off".parse().unwrap());

    // Only show our crate's logs and any errors from other crates
    let _crate_filter = FilterFn::new(|metadata| {
        metadata.target().starts_with("agents") || metadata.level() <= &tracing::Level::ERROR
    });

    let fmt_layer = tracing_subscriber::fmt::layer()
        // .with_target(false) // Don't show target
        .with_thread_ids(false)
        .with_thread_names(false)
        // .with_file(false)
        // .with_line_number(false)
        .with_span_events(FmtSpan::NONE)
        .compact() // Use compact format
        .event_format(
            tracing_subscriber::fmt::format()
                .compact()
                .without_time()
                .with_ansi(true), // .with_target(false)
                                  // .with_level(false),
        )
        .with_ansi(true) // Enable colors
        .with_timer(tracing_subscriber::fmt::time::time());

    tracing_subscriber::registry()
        .with(fmt_layer.with_filter(filter))
        .init();
}

#[macro_export]
macro_rules! verbose_log {
    ($verbose:expr, $($arg:tt)*) => {
        if $verbose {
            tracing::info!($($arg)*);
        }
    };
}

#[cfg(feature = "otel")]
pub fn init_tracer_provider() {
    use opentelemetry::global;
    use opentelemetry_otlp::SpanExporter;

    use opentelemetry_sdk::{
        propagation::TraceContextPropagator, trace::SdkTracerProvider, Resource,
    };
    let exporter = SpanExporter::builder()
        .with_tonic()
        .build()
        .expect("Failed to create span exporter");
    let provider = SdkTracerProvider::builder()
        .with_resource(Resource::builder().with_service_name("distri").build())
        .with_batch_exporter(exporter)
        .build();
    global::set_text_map_propagator(TraceContextPropagator::new());
    global::set_tracer_provider(provider);
}
