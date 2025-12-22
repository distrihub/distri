use distri_types::InlineHookRequest;
use std::future::Future;
use std::sync::Arc;
use std::{collections::HashMap, pin::Pin};

/// A per-execution hook registry that lives alongside an ExecutorContext.
/// Hooks are fire-and-forget - they execute but don't return values.
#[derive(Clone, Default)]
pub struct HookRegistry {
    handlers: Arc<std::sync::RwLock<HashMap<String, Handler>>>,
}

pub type Handler = Arc<dyn Fn(&InlineHookRequest) -> HandlerFuture + Send + Sync + 'static>;
pub type HandlerFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(std::sync::RwLock::new(HashMap::new())),
        }
    }

    pub fn register<F, Fut>(&self, agent: impl Into<String>, handler: F)
    where
        F: Fn(&InlineHookRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if let Ok(mut guard) = self.handlers.write() {
            let wrapped = Arc::new(move |req: &InlineHookRequest| -> HandlerFuture {
                Box::pin(handler(req))
            });
            guard.insert(agent.into(), wrapped);
        }
    }

    pub fn merge_from(&self, other: &HookRegistry) {
        if let (Ok(mut ours), Ok(theirs)) = (self.handlers.write(), other.handlers.read()) {
            for (k, v) in theirs.iter() {
                ours.insert(k.clone(), v.clone());
            }
        }
    }

    /// Execute a hook handler if one exists (fire-and-forget).
    pub async fn try_handle(&self, agent: &str, request: &InlineHookRequest) {
        // Drop the read lock before awaiting the handler to keep the type Send.
        let handler = {
            let guard = match self.handlers.read() {
                Ok(g) => g,
                Err(_) => return,
            };
            match guard.get(agent) {
                Some(h) => h.clone(),
                None => return,
            }
        };
        handler(request).await;
    }
}

