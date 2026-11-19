//! Durable-run stores: [`RunQueue`](distri_types::RunQueue) +
//! [`PendingCallStore`](distri_types::PendingCallStore) implementations.

pub mod conformance;
mod diesel;
mod in_memory;

pub use diesel::DieselDurableStore;
pub use in_memory::{InMemoryDurableStore, InProcessWakeNotifier};

use std::sync::Arc;

use distri_types::durable::{PendingCallStore, RunQueue};

/// The Postgres DDL for the OSS Diesel schema (the sqlite twin is an embedded
/// migration). A host that runs distri-stores on Postgres applies it once.
pub const PG_DURABLE_RUNS_UP_SQL: &str =
    include_str!("../../pg_migrations/20261120000000_durable_runs/up.sql");

/// One object implementing both durable traits.
#[derive(Clone)]
pub struct DurableStores {
    pub run_queue: Arc<dyn RunQueue>,
    pub pending_calls: Arc<dyn PendingCallStore>,
}

impl DurableStores {
    pub fn in_memory() -> Self {
        let store = Arc::new(InMemoryDurableStore::new());
        Self {
            run_queue: store.clone(),
            pending_calls: store,
        }
    }

    pub fn from_store<S>(store: Arc<S>) -> Self
    where
        S: RunQueue + PendingCallStore + 'static,
    {
        Self {
            run_queue: store.clone(),
            pending_calls: store,
        }
    }
}

/// Durable stores over the database holding `config`'s tasks (the session
/// store) on sqlite builds with persistent sessions; in memory otherwise
/// (ephemeral sessions, other backends).
pub async fn initialize_durable_stores(
    config: &distri_types::configuration::StoreConfig,
) -> anyhow::Result<DurableStores> {
    #[cfg(feature = "sqlite")]
    {
        if !config.session.ephemeral
            && matches!(
                config.session.store_type,
                distri_types::configuration::StoreType::Sqlite
            )
        {
            let db = config.session.db_config.clone().unwrap_or_default();
            let builder = crate::diesel_store::SqliteStoreBuilder::sqlite(
                &db.database_url,
                db.max_connections,
            )
            .await?;
            return Ok(DurableStores::from_store(Arc::new(builder.durable_store())));
        }
    }
    let _ = config;
    Ok(DurableStores::in_memory())
}
