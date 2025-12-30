use diesel::connection::{Instrumentation, InstrumentationEvent, set_default_instrumentation};
use tracing::{debug, info, warn};

fn diesel_instrumentation() -> Option<Box<dyn Instrumentation>> {
    Some(Box::new(|event: InstrumentationEvent<'_>| match event {
        InstrumentationEvent::StartQuery { query, .. } => {
            info!(target: "diesel", sql = %query);
        }
        InstrumentationEvent::FinishQuery { query, error, .. } => {
            if let Some(e) = error {
                warn!(target: "diesel", sql = %query, error = %e, "diesel query failed");
            } else {
                debug!(target: "diesel", sql = %query, "diesel query done");
            }
        }
        _ => {}
    }))
}

pub fn init_diesel_instrumentation() {
    // Safe to ignore the result; instrumentation is best-effort.
    let _ = set_default_instrumentation(diesel_instrumentation);
}
