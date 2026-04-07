use anyhow::{Context, Result};
use distri::{Distri, DistriConfig};
use serde::Deserialize;
use std::path::PathBuf;
use tokio::sync::oneshot;
use warp::Filter;

/// Query parameters from the callback
#[derive(Debug, Deserialize)]
struct CallbackQuery {
    api_key: String,
    workspace_id: String,
    state: String,
}

/// Handle the login command flow.
///
/// This function:
/// 1. Gets the login URL from the API server
/// 2. Starts a local HTTP server to listen for the callback
/// 3. Opens the browser to the Distri Cloud login page
/// 4. User logs in on the web and selects a workspace
/// 5. Web redirects back to localhost with credentials
/// 6. Saves the API key and workspace_id to ~/.distri/config
pub async fn handle_login_command(_email: Option<String>, _skip_workspace: bool) -> Result<()> {
    // Get the login URL from the API server first
    println!("Connecting to Distri Cloud...");
    let client = Distri::from_env();
    let login_url_response = client
        .get_login_url()
        .await
        .context("Failed to get login URL from server. Is the server running?")?;

    // Generate a random state token for CSRF protection
    let state = uuid::Uuid::new_v4().to_string();

    // Channel to pass credentials from callback handler to main task
    let (cred_tx, cred_rx) = oneshot::channel::<(String, String)>();
    let cred_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(cred_tx)));

    // Channel to signal server shutdown after credentials are received
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let shutdown_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(shutdown_tx)));

    let state_clone = state.clone();

    // Create the callback route
    let callback_route = warp::path("callback")
        .and(warp::query::<CallbackQuery>())
        .map(move |query: CallbackQuery| {
            let expected_state = state_clone.clone();
            let cred_tx = cred_tx.clone();
            let shutdown_tx = shutdown_tx.clone();

            if query.state != expected_state {
                eprintln!("State token mismatch - possible CSRF attack");
                return warp::reply::html(
                    r#"<!DOCTYPE html><html><body>Authentication failed: state mismatch.</body></html>"#,
                );
            }

            // Send credentials and trigger shutdown — take each sender at most once
            if let Some(tx) = cred_tx.lock().unwrap().take() {
                let _ = tx.send((query.api_key, query.workspace_id));
            }
            if let Some(tx) = shutdown_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }

            warp::reply::html(
                r#"
                <!DOCTYPE html>
                <html>
                <head><title>Login Successful</title></head>
                <body style="font-family: system-ui; text-align: center; padding: 50px;">
                    <h1>✓ Login Successful</h1>
                    <p>You can close this window and return to the terminal.</p>
                </body>
                </html>
                "#,
            )
        });

    // Bind to port 0 so the OS assigns an available port, then get the actual address.
    // This avoids the TOCTOU race of find-available-port + rebind.
    let (addr, server) = warp::serve(callback_route).bind_with_graceful_shutdown(
        ([127, 0, 0, 1], 0u16),
        async move {
            let _ = shutdown_rx.await;
        },
    );

    // Now we know the actual port — construct the callback URL AFTER binding.
    let callback_url = format!("http://localhost:{}", addr.port());

    // Construct the full login URL with callback and state
    let login_url = format!(
        "{}?callback={}&state={}",
        login_url_response.login_url,
        urlencoding::encode(&callback_url),
        urlencoding::encode(&state)
    );

    println!("Opening browser for authentication...");
    println!("Waiting for callback on {}...", addr);

    // Open the browser — server is already bound and accepting connections.
    if let Err(e) = open_browser(&login_url) {
        eprintln!("Failed to open browser: {}", e);
        println!("\nPlease open this URL in your browser:");
        println!("{}", login_url);
    }

    // Drive the server concurrently with waiting for credentials
    let (_, creds) = tokio::join!(server, async {
        cred_rx.await.context("Login was cancelled or timed out")
    });
    let (api_key, workspace_id) = creds?;

    // Save config
    save_config(&api_key, &workspace_id)?;

    println!("\n✓ Successfully authenticated!");
    println!(
        "  API Key: {}...{}",
        &api_key[..10],
        &api_key[api_key.len() - 4..]
    );
    println!("  Workspace ID: {}", workspace_id);
    println!("\nYou can now use 'distri'. Run 'distri -h' for help");

    Ok(())
}

/// Open the system default browser with the given URL
fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(&["/C", "start", url])
            .spawn()?;
    }

    Ok(())
}

/// Save the API key and workspace_id to ~/.distri/config.
fn save_config(api_key: &str, workspace_id: &str) -> Result<()> {
    let path = DistriConfig::config_path().context("Unable to resolve ~/.distri/config path")?;

    // Create parent directory
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Load existing config or create new
    let mut config = load_config_toml(&path);

    // Update values
    if let toml::Value::Table(ref mut table) = config {
        table.insert(
            "api_key".to_string(),
            toml::Value::String(api_key.to_string()),
        );
        table.insert(
            "workspace_id".to_string(),
            toml::Value::String(workspace_id.to_string()),
        );
    }

    // Write to file
    let contents = toml::to_string_pretty(&config)?;
    std::fs::write(&path, contents)?;

    Ok(())
}

/// Load the existing config file as a TOML value, or create an empty table.
fn load_config_toml(path: &PathBuf) -> toml::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| contents.parse::<toml::Value>().ok())
        .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()))
}
