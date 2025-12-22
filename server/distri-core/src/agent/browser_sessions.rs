use browsr_client::{default_transport, BrowsrClient};
use dashmap::DashMap;
use distri_types::{browser::DistriBrowserConfig, OrchestratorRef, OrchestratorTrait};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, RwLock};

#[derive(Clone)]
pub struct BrowserSessionEntry {
    pub session_id: String,
    pub last_used: Instant,
}

#[derive(Clone)]
pub struct BrowserSessions {
    sessions: Arc<DashMap<String, BrowserSessionEntry>>,
    orchestrator_ref: Arc<OrchestratorRef>,
    client: BrowsrClient,
}

impl BrowserSessions {
    pub fn new(_config: Arc<RwLock<DistriBrowserConfig>>) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            orchestrator_ref: Arc::new(OrchestratorRef::new()),
            client: BrowsrClient::from_config(default_transport()),
        }
    }
    pub fn set_orchestrator(&self, orchestrator: Arc<dyn OrchestratorTrait>) {
        self.orchestrator_ref.set_orchestrator(orchestrator.clone());
    }

    async fn build_browser(
        &self,
        _headless: Option<bool>,
        _start_url: Option<String>,
    ) -> Result<String, String> {
        self.client
            .create_session()
            .await
            .map_err(|e| format!("Failed to initialize browser session: {}", e))
    }

    pub async fn create(
        &self,
        headless: Option<bool>,
        requested_name: Option<String>,
        start_url: Option<String>,
    ) -> Result<(String, Arc<Mutex<()>>), String> {
        let session_id = self.build_browser(headless, start_url).await?;

        let name = requested_name
            .and_then(|s| {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
            .unwrap_or_else(|| session_id.clone());

        if self.sessions.contains_key(&name) {
            return Ok((name, Arc::new(Mutex::new(()))));
        }

        self.sessions.insert(
            name.clone(),
            BrowserSessionEntry {
                session_id: session_id.clone(),
                last_used: Instant::now(),
            },
        );

        Ok((name, Arc::new(Mutex::new(()))))
    }

    pub async fn ensure(
        &self,
        requested: Option<String>,
        headless: Option<bool>,
        start_url: Option<String>,
    ) -> Result<(String, Arc<Mutex<()>>), String> {
        if let Some(id) = requested {
            if let Some(mut entry) = self.sessions.get_mut(&id) {
                entry.last_used = Instant::now();
                return Ok((id, Arc::new(Mutex::new(()))));
            }
            return Err(format!("Session {} not found", id));
        }

        if let Some(mut entry) = self.sessions.iter_mut().next() {
            entry.last_used = Instant::now();
            return Ok((entry.key().clone(), Arc::new(Mutex::new(()))));
        }

        self.create(headless, None, start_url).await
    }

    pub fn list(&self) -> Vec<String> {
        self.sessions
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    pub fn stop(&self, id: &str) -> bool {
        if let Some((_, entry)) = self.sessions.remove(id) {
            let _ = self.client.destroy_session(&entry.session_id);
            true
        } else {
            false
        }
    }

    pub fn client(&self) -> BrowsrClient {
        self.client.clone()
    }

    pub fn session_id_for(&self, name: &str) -> Option<String> {
        self.sessions.get(name).map(|e| e.session_id.clone())
    }
}
