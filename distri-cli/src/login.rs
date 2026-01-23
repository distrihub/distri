use anyhow::{Context, Result};
use distri::{Distri, DistriConfig};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use warp::Filter;

/// Credentials received from the web callback
#[derive(Debug, Clone)]
struct LoginCredentials {
    api_key: String,
    workspace_id: String,
}

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

    // Try to find an available port
    let port = find_available_port().await?;
    let callback_url = format!("http://localhost:{}", port);

    // Construct the full login URL with callback and state
    let login_url = format!(
        "{}?callback={}&state={}",
        login_url_response.login_url,
        urlencoding::encode(&callback_url),
        urlencoding::encode(&state)
    );

    // Shared state for receiving credentials
    let credentials: Arc<Mutex<Option<LoginCredentials>>> = Arc::new(Mutex::new(None));
    let credentials_clone = credentials.clone();
    let state_clone = state.clone();

    // Create the callback route
    let callback_route = warp::path("callback")
        .and(warp::query::<CallbackQuery>())
        .map(move |query: CallbackQuery| {
            let credentials = credentials_clone.clone();
            let expected_state = state_clone.clone();

            tokio::spawn(async move {
                // Verify state token
                if query.state != expected_state {
                    eprintln!("State token mismatch - possible CSRF attack");
                    return;
                }

                // Store credentials
                let creds = LoginCredentials {
                    api_key: query.api_key,
                    workspace_id: query.workspace_id,
                };

                let mut lock = credentials.lock().await;
                *lock = Some(creds);
            });

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

    // Start the server in the background
    let credentials_shutdown = credentials.clone();
    let (addr, server) =
        warp::serve(callback_route).bind_with_graceful_shutdown(([127, 0, 0, 1], port), async move {
            // Wait until we receive credentials
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                let lock = credentials_shutdown.lock().await;
                if lock.is_some() {
                    break;
                }
            }
        });

    println!("Opening browser for authentication...");
    println!("Waiting for callback on {}...", addr);

    // Open the browser
    if let Err(e) = open_browser(&login_url) {
        eprintln!("Failed to open browser: {}", e);
        println!("\nPlease open this URL in your browser:");
        println!("{}", login_url);
    }

    // Run the server and wait for callback
    server.await;

    // Retrieve credentials
    let creds = credentials
        .lock()
        .await
        .clone()
        .context("Login was cancelled or timed out")?;

    // Save config
    save_config(&creds.api_key, &creds.workspace_id)?;

    println!("\n✓ Successfully authenticated!");
    println!("  API Key: {}...{}", &creds.api_key[..10], &creds.api_key[creds.api_key.len() - 4..]);
    println!("  Workspace ID: {}", creds.workspace_id);
    println!("\nYou can now use 'distri run' and other commands.");

    Ok(())
}

/// Find an available port for the callback server
async fn find_available_port() -> Result<u16> {
    for port in 7878..7900 {
        if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            drop(listener);
            return Ok(port);
        }
    }
    anyhow::bail!("Could not find an available port for the callback server")
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
        table.insert("api_key".to_string(), toml::Value::String(api_key.to_string()));
        table.insert("workspace_id".to_string(), toml::Value::String(workspace_id.to_string()));
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
