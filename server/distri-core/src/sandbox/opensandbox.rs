//! [`SandboxProvider`] for OpenSandbox (<https://github.com/opensandbox-group/OpenSandbox>).
//!
//! Two APIs are used:
//! - the lifecycle API on the OpenSandbox server (`/v1/sandboxes…`), with the
//!   `OPEN-SANDBOX-API-KEY` header;
//! - `execd` inside each sandbox (port 44772), reached through the endpoint
//!   the lifecycle API returns (`/v1/sandboxes/{id}/endpoints/44772`), for
//!   commands (`POST /command`, a stream of JSON events) and files
//!   (`/files/upload`, `/files/download`).
//!
//! Configuration comes from the environment (see [`OpenSandboxConfig::from_env`]).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    ExecChunk, ExecOutput, ExecRequest, SandboxError, SandboxInfo, SandboxProvider, SandboxResult,
    SandboxSpec, SandboxState, WORKSPACE_DIR,
};

/// The port `execd` listens on inside every OpenSandbox sandbox.
pub const EXECD_PORT: u16 = 44772;

/// How long a data-plane call keeps retrying while `execd` is not reachable
/// yet (just after create or resume).
const EXECD_READY_TIMEOUT: Duration = Duration::from_secs(30);

/// The header the ingress gateway puts on an endpoint it serves.
const INGRESS_HEADER: &str = "OpenSandbox-Ingress-To";

#[derive(Clone)]
pub struct OpenSandboxConfig {
    /// Lifecycle API base, ending in `/v1`.
    pub base_url: String,
    pub api_key: Option<String>,
    /// Image used when a [`SandboxSpec`] leaves it empty.
    pub default_image: String,
    /// Reach `execd` through the server's proxy instead of the direct
    /// endpoint (needed when only the server is reachable).
    pub use_server_proxy: bool,
    pub cpu: String,
    pub memory: String,
    /// Guaranteed share (`resourceRequests`); the limits above are the cap.
    /// Without it a sandbox reserves its whole limit, and a 2-vCPU node
    /// holds one 1-CPU sandbox.
    pub cpu_request: Option<String>,
    pub memory_request: Option<String>,
    pub default_timeout_secs: Option<u64>,
    /// Ask for `secureAccess`: execd then needs the per-sandbox token the
    /// endpoint call returns. Required on sandbox-api.distri.dev, where
    /// any allowlisted caller could otherwise reach a sandbox's ports.
    pub secure_access: bool,
    /// Size of a thread's volume when it is created (`5Gi`).
    pub volume_size: Option<String>,
}

impl std::fmt::Debug for OpenSandboxConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenSandboxConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("default_image", &self.default_image)
            .field("use_server_proxy", &self.use_server_proxy)
            .field("secure_access", &self.secure_access)
            .finish()
    }
}

fn env_first(names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|n| std::env::var(n).ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl OpenSandboxConfig {
    /// Reads:
    /// - `SANDBOX_URL` (or `OPENSANDBOX_URL`): the server, e.g.
    ///   `https://sandbox-api.distri.dev` or `http://localhost:8090`;
    /// - `SANDBOX_API_KEY` (or `OPENSANDBOX_API_KEY`): OpenSandbox's key;
    /// - `OPENSANDBOX_IMAGE` (default `python:3.12-slim`);
    /// - `SANDBOX_USE_SERVER_PROXY` (`true`/`1`);
    /// - `OPENSANDBOX_CPU` (default `1`), `OPENSANDBOX_MEMORY` (default `1Gi`),
    ///   `OPENSANDBOX_TIMEOUT_SECS` (default 3600; a sandbox that expired is
    ///   recreated on its volume at the next tool call);
    /// - `OPENSANDBOX_SECURE_ACCESS` (default on, except for a localhost URL:
    ///   OpenSandbox's Docker mode has no ingress gateway to enforce it);
    /// - `OPENSANDBOX_VOLUME_SIZE` (default `5Gi`; ignored by Docker volumes);
    /// - `OPENSANDBOX_CPU_REQUEST` (default `250m`), `OPENSANDBOX_MEMORY_REQUEST`
    ///   (default `512Mi`): the reservation, below the limits (k8s only).
    ///
    /// `SANDBOX_IMAGE` belongs to the old browsr launcher and is not read.
    pub fn from_env() -> SandboxResult<Self> {
        let url = env_first(&["SANDBOX_URL", "OPENSANDBOX_URL"])
            .ok_or_else(|| SandboxError::NotConfigured("SANDBOX_URL is not set".into()))?;
        let flag = |n: &str, default: bool| {
            env_first(&[n])
                .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(default)
        };
        let local = reqwest::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h == "localhost" || h == "127.0.0.1"))
            .unwrap_or(false);
        Ok(Self {
            base_url: normalize_base(&url),
            api_key: env_first(&["SANDBOX_API_KEY", "OPENSANDBOX_API_KEY"]),
            default_image: env_first(&["OPENSANDBOX_IMAGE"])
                .unwrap_or_else(|| "python:3.12-slim".to_string()),
            use_server_proxy: flag("SANDBOX_USE_SERVER_PROXY", false),
            secure_access: flag("OPENSANDBOX_SECURE_ACCESS", !local),
            volume_size: Some(
                env_first(&["OPENSANDBOX_VOLUME_SIZE"]).unwrap_or_else(|| "5Gi".to_string()),
            ),
            cpu: env_first(&["OPENSANDBOX_CPU"]).unwrap_or_else(|| "1".into()),
            memory: env_first(&["OPENSANDBOX_MEMORY"]).unwrap_or_else(|| "1Gi".into()),
            cpu_request: Some(
                env_first(&["OPENSANDBOX_CPU_REQUEST"]).unwrap_or_else(|| "250m".into()),
            ),
            memory_request: Some(
                env_first(&["OPENSANDBOX_MEMORY_REQUEST"]).unwrap_or_else(|| "512Mi".into()),
            ),
            default_timeout_secs: Some(
                env_first(&["OPENSANDBOX_TIMEOUT_SECS"])
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(3_600),
            ),
        })
    }
}

fn normalize_base(url: &str) -> String {
    let url = url.trim_end_matches('/');
    if url.ends_with("/v1") {
        url.to_string()
    } else {
        format!("{url}/v1")
    }
}

#[derive(Debug, Clone)]
struct Endpoint {
    url: String,
    headers: Vec<(String, String)>,
}

#[derive(Debug)]
pub struct OpenSandboxProvider {
    config: OpenSandboxConfig,
    http: reqwest::Client,
    endpoints: DashMap<String, Endpoint>,
}

impl OpenSandboxProvider {
    pub fn new(config: OpenSandboxConfig) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client");
        Self {
            config,
            http,
            endpoints: DashMap::new(),
        }
    }

    pub fn from_env() -> SandboxResult<Arc<Self>> {
        Ok(Arc::new(Self::new(OpenSandboxConfig::from_env()?)))
    }

    pub fn config(&self) -> &OpenSandboxConfig {
        &self.config
    }

    fn lifecycle(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut rb = self
            .http
            .request(method, format!("{}{}", self.config.base_url, path));
        if let Some(key) = &self.config.api_key {
            rb = rb.header("OPEN-SANDBOX-API-KEY", key);
        }
        rb
    }

    async fn endpoint(&self, sandbox_id: &str) -> SandboxResult<Endpoint> {
        if let Some(ep) = self.endpoints.get(sandbox_id) {
            return Ok(ep.clone());
        }
        let mut path = format!("/sandboxes/{sandbox_id}/endpoints/{EXECD_PORT}");
        if self.config.use_server_proxy {
            path.push_str("?use_server_proxy=true");
        }
        let resp = self
            .lifecycle(reqwest::Method::GET, &path)
            .send()
            .await
            .map_err(transport)?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SandboxError::NotFound(sandbox_id.to_string()));
        }
        let body: EndpointResponse = check(resp).await?.json().await.map_err(transport)?;
        // The endpoint's own headers (ingress routing, secure-access token).
        let mut headers: Vec<(String, String)> = body.headers.into_iter().collect();
        let via_gateway = headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case(INGRESS_HEADER));
        let url = self.endpoint_url(&body.endpoint, via_gateway);
        if self.config.use_server_proxy {
            // The proxied endpoint is the server itself: it wants the API key.
            if let Some(key) = &self.config.api_key {
                headers.push(("OPEN-SANDBOX-API-KEY".into(), key.clone()));
            }
        }
        let ep = Endpoint { url, headers };
        self.endpoints.insert(sandbox_id.to_string(), ep.clone());
        Ok(ep)
    }

    /// OpenSandbox returns endpoints without a scheme (`127.0.0.1:57487/proxy/44772`).
    ///
    /// An endpoint served by the ingress gateway (it carries
    /// `OpenSandbox-Ingress-To`) is reached through `SANDBOX_URL`'s own origin,
    /// whatever host the server names: one gateway answers on several names
    /// (sandbox-api.distri.dev for production, sandbox-internal.distri.dev for
    /// staging), and a caller must stay on the one it was given.
    fn endpoint_url(&self, raw: &str, via_gateway: bool) -> String {
        let raw = raw.trim_end_matches('/');
        let base = reqwest::Url::parse(&self.config.base_url).ok();
        if via_gateway {
            if let Some(origin) = base.as_ref().map(|u| u.origin().ascii_serialization()) {
                let rest = raw.split_once("://").map(|(_, r)| r).unwrap_or(raw);
                let path = rest.find('/').map(|i| &rest[i..]).unwrap_or("");
                return format!("{origin}{path}");
            }
        }
        if raw.starts_with("http://") || raw.starts_with("https://") {
            return raw.to_string();
        }
        let same_host = base
            .as_ref()
            .and_then(|u| u.host_str().map(|h| raw.starts_with(h)))
            .unwrap_or(false);
        let scheme = if same_host {
            base.map(|u| u.scheme().to_string())
                .unwrap_or_else(|| "http".into())
        } else {
            "http".to_string()
        };
        format!("{scheme}://{raw}")
    }

    fn data(&self, ep: &Endpoint, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut rb = self.http.request(method, format!("{}{}", ep.url, path));
        for (k, v) in &ep.headers {
            rb = rb.header(k, v);
        }
        rb
    }

    /// Send a data-plane request, retrying while `execd` is not reachable
    /// (connection refused just after create/resume) and re-resolving the
    /// endpoint, which can change when a sandbox is resumed.
    async fn send_data<F>(&self, sandbox_id: &str, build: F) -> SandboxResult<reqwest::Response>
    where
        F: Fn(&Endpoint) -> reqwest::RequestBuilder,
    {
        let deadline = tokio::time::Instant::now() + EXECD_READY_TIMEOUT;
        let mut delay = Duration::from_millis(200);
        loop {
            let ep = self.endpoint(sandbox_id).await?;
            match build(&ep).send().await {
                Ok(resp)
                    if matches!(resp.status().as_u16(), 502..=504)
                        && tokio::time::Instant::now() < deadline =>
                {
                    tracing::debug!(sandbox_id, status = %resp.status(), "execd not ready");
                }
                Ok(resp) => return Ok(resp),
                Err(e)
                    if (e.is_connect() || e.is_request())
                        && tokio::time::Instant::now() < deadline =>
                {
                    tracing::debug!(sandbox_id, error = %e, "execd not reachable yet");
                }
                Err(e) => return Err(transport(e)),
            }
            self.endpoints.remove(sandbox_id);
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(2));
        }
    }
}

#[derive(Deserialize)]
struct EndpointResponse {
    endpoint: String,
    #[serde(default)]
    headers: HashMap<String, String>,
}

#[derive(Deserialize)]
struct SandboxResponse {
    id: String,
    status: Option<StatusResponse>,
}

#[derive(Deserialize)]
struct StatusResponse {
    state: String,
    #[serde(default)]
    message: Option<String>,
}

fn map_state(s: &str) -> SandboxState {
    match s {
        "Pending" => SandboxState::Pending,
        "Running" => SandboxState::Running,
        "Pausing" => SandboxState::Pausing,
        "Paused" => SandboxState::Paused,
        "Resuming" => SandboxState::Resuming,
        "Stopping" => SandboxState::Stopping,
        // Terminated, Failed, and anything unknown: treat as gone.
        _ => SandboxState::Gone,
    }
}

fn transport(e: reqwest::Error) -> SandboxError {
    SandboxError::Provider(format!("opensandbox request failed: {e}"))
}

async fn check(resp: reqwest::Response) -> SandboxResult<reqwest::Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    Err(SandboxError::Provider(format!(
        "opensandbox returned {status}: {}",
        body.chars().take(500).collect::<String>()
    )))
}

impl From<SandboxResponse> for SandboxInfo {
    fn from(r: SandboxResponse) -> Self {
        let (state, message) = match r.status {
            Some(s) => (map_state(&s.state), s.message),
            None => (SandboxState::Pending, None),
        };
        SandboxInfo {
            id: r.id,
            state,
            message,
        }
    }
}

/// Folds `execd`'s command events into an [`ExecOutput`].
#[derive(Default)]
struct EventFold {
    out: ExecOutput,
    exit: Option<i64>,
}

fn push_line(buf: &mut String, text: &str) {
    buf.push_str(text);
    // execd v1.1 sends one event per line without its line ending; newer
    // versions keep the ending. Normalise to one line per event.
    if !text.ends_with('\n') && !text.ends_with('\r') {
        buf.push('\n');
    }
}

impl EventFold {
    fn apply(
        &mut self,
        ev: &Value,
        on_chunk: &Option<tokio::sync::mpsc::UnboundedSender<ExecChunk>>,
    ) {
        let text = ev.get("text").and_then(|t| t.as_str()).unwrap_or("");
        match ev.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "stdout" => {
                push_line(&mut self.out.stdout, text);
                if let Some(tx) = on_chunk {
                    let _ = tx.send(ExecChunk::Stdout(text.to_string()));
                }
            }
            "stderr" => {
                push_line(&mut self.out.stderr, text);
                if let Some(tx) = on_chunk {
                    let _ = tx.send(ExecChunk::Stderr(text.to_string()));
                }
            }
            "execution_complete" => {
                if self.exit.is_none() {
                    self.exit = Some(ev.get("exit_code").and_then(|c| c.as_i64()).unwrap_or(0));
                }
            }
            "error" => {
                let err = ev.get("error").cloned().unwrap_or(Value::Null);
                let ename = err.get("ename").and_then(|v| v.as_str()).unwrap_or("");
                if ename == "CommandTimeout" {
                    self.out.timed_out = true;
                    self.exit = Some(-1);
                    return;
                }
                let code = err.get("exit_code").and_then(|v| v.as_i64()).or_else(|| {
                    err.get("evalue")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.trim().parse().ok())
                });
                match code {
                    Some(c) => self.exit = Some(c),
                    None => {
                        let evalue = err.get("evalue").and_then(|v| v.as_str()).unwrap_or("");
                        push_line(&mut self.out.stderr, &format!("{ename}: {evalue}"));
                        self.exit = Some(-1);
                    }
                }
            }
            _ => {}
        }
    }

    /// Accepts both raw JSON objects and SSE `data:` lines.
    fn apply_line(
        &mut self,
        line: &str,
        on_chunk: &Option<tokio::sync::mpsc::UnboundedSender<ExecChunk>>,
    ) {
        let line = line.trim();
        let line = line.strip_prefix("data:").map(str::trim).unwrap_or(line);
        if line.is_empty() {
            return;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(ev) => self.apply(&ev, on_chunk),
            Err(_) => tracing::debug!(line, "ignoring non-JSON execd line"),
        }
    }

    fn finish(mut self) -> ExecOutput {
        self.out.exit_code = self.exit.unwrap_or(-1);
        self.out
    }
}

#[async_trait::async_trait]
impl SandboxProvider for OpenSandboxProvider {
    fn name(&self) -> &'static str {
        "opensandbox"
    }

    async fn create(&self, spec: &SandboxSpec) -> SandboxResult<SandboxInfo> {
        let image = if spec.image.is_empty() {
            self.config.default_image.clone()
        } else {
            spec.image.clone()
        };
        let mut body = json!({
            "image": { "uri": image },
            "entrypoint": ["tail", "-f", "/dev/null"],
            "resourceLimits": { "cpu": self.config.cpu, "memory": self.config.memory },
            "env": spec.env,
            "metadata": spec.metadata,
        });
        if let Some(t) = spec.timeout_secs.or(self.config.default_timeout_secs) {
            body["timeout"] = json!(t);
        }
        if self.config.secure_access {
            body["secureAccess"] = json!(true);
        }
        let mut requests = serde_json::Map::new();
        if let Some(c) = &self.config.cpu_request {
            requests.insert("cpu".into(), json!(c));
        }
        if let Some(m) = &self.config.memory_request {
            requests.insert("memory".into(), json!(m));
        }
        if !requests.is_empty() {
            body["resourceRequests"] = Value::Object(requests);
        }
        if let Some(volume) = &spec.volume {
            let mut pvc = json!({ "claimName": volume, "createIfNotExists": true });
            if let Some(size) = &self.config.volume_size {
                pvc["storage"] = json!(size);
            }
            body["volumes"] = json!([{
                "name": "workspace",
                "pvc": pvc,
                "mountPath": WORKSPACE_DIR,
            }]);
        }
        let resp = self
            .lifecycle(reqwest::Method::POST, "/sandboxes")
            .json(&body)
            .send()
            .await
            .map_err(transport)?;
        let created: SandboxResponse = check(resp).await?.json().await.map_err(transport)?;
        Ok(created.into())
    }

    async fn status(&self, sandbox_id: &str) -> SandboxResult<SandboxInfo> {
        let resp = self
            .lifecycle(reqwest::Method::GET, &format!("/sandboxes/{sandbox_id}"))
            .send()
            .await
            .map_err(transport)?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(SandboxInfo {
                id: sandbox_id.to_string(),
                state: SandboxState::Gone,
                message: Some("not found".into()),
            });
        }
        let info: SandboxResponse = check(resp).await?.json().await.map_err(transport)?;
        Ok(info.into())
    }

    async fn pause(&self, sandbox_id: &str) -> SandboxResult<()> {
        self.endpoints.remove(sandbox_id);
        let resp = self
            .lifecycle(
                reqwest::Method::POST,
                &format!("/sandboxes/{sandbox_id}/pause"),
            )
            .send()
            .await
            .map_err(transport)?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SandboxError::NotFound(sandbox_id.to_string()));
        }
        check(resp).await.map(|_| ())
    }

    async fn resume(&self, sandbox_id: &str) -> SandboxResult<()> {
        self.endpoints.remove(sandbox_id);
        let resp = self
            .lifecycle(
                reqwest::Method::POST,
                &format!("/sandboxes/{sandbox_id}/resume"),
            )
            .send()
            .await
            .map_err(transport)?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SandboxError::NotFound(sandbox_id.to_string()));
        }
        check(resp).await.map(|_| ())
    }

    async fn delete(&self, sandbox_id: &str) -> SandboxResult<()> {
        self.endpoints.remove(sandbox_id);
        let resp = self
            .lifecycle(reqwest::Method::DELETE, &format!("/sandboxes/{sandbox_id}"))
            .send()
            .await
            .map_err(transport)?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        check(resp).await.map(|_| ())
    }

    async fn find_by_metadata(&self, key: &str, value: &str) -> SandboxResult<Vec<SandboxInfo>> {
        // `metadata` is one url-encoded `k=v` (OpenSandbox's list filter).
        let resp = self
            .lifecycle(reqwest::Method::GET, "/sandboxes")
            .query(&[
                ("metadata", format!("{key}={value}")),
                ("pageSize", "50".into()),
            ])
            .send()
            .await
            .map_err(transport)?;
        #[derive(Deserialize)]
        struct ListItem {
            id: String,
            status: Option<StatusResponse>,
            #[serde(default, rename = "createdAt")]
            created_at: Option<String>,
            #[serde(default)]
            metadata: HashMap<String, String>,
        }
        #[derive(Deserialize)]
        struct ListResponse {
            #[serde(default)]
            items: Vec<ListItem>,
        }
        let list: ListResponse = check(resp).await?.json().await.map_err(transport)?;
        let mut items: Vec<ListItem> = list
            .items
            .into_iter()
            // Do not trust the server to have filtered.
            .filter(|i| i.metadata.get(key).map(String::as_str) == Some(value))
            .collect();
        items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(items
            .into_iter()
            .map(|i| {
                SandboxInfo::from(SandboxResponse {
                    id: i.id,
                    status: i.status,
                })
            })
            .collect())
    }

    async fn wait_ready(&self, sandbox_id: &str, timeout: Duration) -> SandboxResult<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut delay = Duration::from_millis(100);
        loop {
            let ep = self.endpoint(sandbox_id).await?;
            // A short per-try timeout: just after a resume the gateway can
            // hold a request to the old pod for ~30 s before failing it.
            let res = self
                .data(&ep, reqwest::Method::GET, "/ping")
                .timeout(Duration::from_secs(2))
                .send()
                .await;
            match res {
                Ok(r) if r.status().is_success() => return Ok(()),
                Ok(r) => tracing::debug!(sandbox_id, status = %r.status(), "execd not ready"),
                Err(e) => tracing::debug!(sandbox_id, error = %e, "execd not reachable"),
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(SandboxError::Provider(format!(
                    "execd in sandbox {sandbox_id} did not answer within {timeout:?}"
                )));
            }
            self.endpoints.remove(sandbox_id);
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(1));
        }
    }

    async fn exec(
        &self,
        sandbox_id: &str,
        req: &ExecRequest,
        on_chunk: Option<tokio::sync::mpsc::UnboundedSender<ExecChunk>>,
    ) -> SandboxResult<ExecOutput> {
        let mut body = json!({ "command": req.command });
        if let Some(cwd) = &req.cwd {
            body["cwd"] = json!(cwd);
        }
        if !req.env.is_empty() {
            body["envs"] = json!(req.env);
        }
        if let Some(t) = req.timeout {
            body["timeout"] = json!(t.as_millis() as u64);
        }
        let resp = self
            .send_data(sandbox_id, |ep| {
                self.data(ep, reqwest::Method::POST, "/command").json(&body)
            })
            .await?;
        let resp = check(resp).await?;

        let collect = async {
            let mut fold = EventFold::default();
            let mut buf: Vec<u8> = Vec::new();
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(transport)?;
                buf.extend_from_slice(&chunk);
                while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    fold.apply_line(&String::from_utf8_lossy(&line), &on_chunk);
                }
            }
            if !buf.is_empty() {
                fold.apply_line(&String::from_utf8_lossy(&buf), &on_chunk);
            }
            Ok::<_, SandboxError>(fold.finish())
        };

        match req.timeout {
            // execd enforces the timeout; this is a backstop in case the
            // stream never closes.
            Some(t) => match tokio::time::timeout(t + Duration::from_secs(15), collect).await {
                Ok(r) => r,
                Err(_) => Ok(ExecOutput {
                    stdout: String::new(),
                    stderr: format!("command did not finish within {t:?}"),
                    exit_code: -1,
                    timed_out: true,
                }),
            },
            None => collect.await,
        }
    }

    async fn read_file(&self, sandbox_id: &str, path: &str) -> SandboxResult<Vec<u8>> {
        let resp = self
            .send_data(sandbox_id, |ep| {
                self.data(ep, reqwest::Method::GET, "/files/download")
                    .query(&[("path", path)])
            })
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SandboxError::FileNotFound(path.to_string()));
        }
        Ok(check(resp)
            .await?
            .bytes()
            .await
            .map_err(transport)?
            .to_vec())
    }

    async fn write_file(&self, sandbox_id: &str, path: &str, content: &[u8]) -> SandboxResult<()> {
        let metadata = json!({ "path": path, "mode": 644 }).to_string();
        let resp = self
            .send_data(sandbox_id, |ep| {
                // execd reads the metadata as a file part, not a text field.
                let form = reqwest::multipart::Form::new()
                    .part(
                        "metadata",
                        reqwest::multipart::Part::bytes(metadata.clone().into_bytes())
                            .file_name("metadata.json")
                            .mime_str("application/json")
                            .expect("mime"),
                    )
                    .part(
                        "file",
                        reqwest::multipart::Part::bytes(content.to_vec())
                            .file_name("file")
                            .mime_str("application/octet-stream")
                            .expect("mime"),
                    );
                self.data(ep, reqwest::Method::POST, "/files/upload")
                    .multipart(form)
            })
            .await?;
        check(resp).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fold(lines: &[&str]) -> ExecOutput {
        let mut f = EventFold::default();
        for l in lines {
            f.apply_line(l, &None);
        }
        f.finish()
    }

    #[test]
    fn folds_success() {
        let out = fold(&[
            r#"{"type":"init","text":"abc"}"#,
            r#"{"type":"stdout","text":"a"}"#,
            r#"data: {"type":"stdout","text":"b\n"}"#,
            r#"{"type":"stderr","text":"warn"}"#,
            r#"{"type":"execution_complete","execution_time":3}"#,
        ]);
        assert_eq!(out.stdout, "a\nb\n");
        assert_eq!(out.stderr, "warn\n");
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn folds_exit_code_and_timeout() {
        let out = fold(&[
            r#"{"type":"error","error":{"ename":"CommandExecError","evalue":"3","traceback":["exit status 3"]}}"#,
        ]);
        assert_eq!(out.exit_code, 3);
        let out = fold(&[r#"{"type":"error","error":{"ename":"CommandTimeout","evalue":"x"}}"#]);
        assert!(out.timed_out);
        assert_eq!(out.exit_code, -1);
    }

    #[test]
    fn endpoint_scheme() {
        let p = OpenSandboxProvider::new(OpenSandboxConfig {
            base_url: normalize_base("https://sandbox-api.distri.dev"),
            api_key: None,
            default_image: "x".into(),
            use_server_proxy: true,
            cpu: "1".into(),
            memory: "1Gi".into(),
            default_timeout_secs: None,
            secure_access: true,
            volume_size: None,
            cpu_request: None,
            memory_request: None,
        });
        assert_eq!(p.config.base_url, "https://sandbox-api.distri.dev/v1");
        assert_eq!(
            p.endpoint_url("sandbox-api.distri.dev/v1/sandboxes/a/proxy/44772", false),
            "https://sandbox-api.distri.dev/v1/sandboxes/a/proxy/44772"
        );
        assert_eq!(
            p.endpoint_url("sandbox-api.distri.dev", false),
            "https://sandbox-api.distri.dev"
        );
        assert_eq!(
            p.endpoint_url("127.0.0.1:5000/proxy/44772", false),
            "http://127.0.0.1:5000/proxy/44772"
        );
    }

    #[test]
    fn gateway_endpoint_stays_on_the_configured_host() {
        let mut cfg = OpenSandboxConfig {
            base_url: normalize_base("https://sandbox-internal.distri.dev"),
            api_key: None,
            default_image: "x".into(),
            use_server_proxy: false,
            cpu: "1".into(),
            memory: "1Gi".into(),
            default_timeout_secs: None,
            secure_access: true,
            volume_size: None,
            cpu_request: None,
            memory_request: None,
        };
        let p = OpenSandboxProvider::new(cfg.clone());
        assert_eq!(
            p.endpoint_url("sandbox-api.distri.dev", true),
            "https://sandbox-internal.distri.dev"
        );
        assert_eq!(
            p.endpoint_url("https://sandbox-api.distri.dev/gw/x", true),
            "https://sandbox-internal.distri.dev/gw/x"
        );
        cfg.base_url = normalize_base("https://sandbox-api.distri.dev");
        let p = OpenSandboxProvider::new(cfg);
        assert_eq!(
            p.endpoint_url("sandbox-api.distri.dev", true),
            "https://sandbox-api.distri.dev"
        );
    }
}
