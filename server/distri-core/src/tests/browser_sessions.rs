use browsr_client::BrowsrClient;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::agent::browser_sessions::BrowserSessions;

// ── Helpers ────────────────────────────────────────────────────────────────

/// Start a wiremock server and return a BrowserSessions wired to it.
async fn setup() -> (MockServer, BrowserSessions) {
    let mock_server = MockServer::start().await;
    let client = BrowsrClient::new(mock_server.uri());
    let sessions = BrowserSessions::new_with_client(client);
    (mock_server, sessions)
}

/// Mount a mock for `POST /sessions` that returns a session with the given ID.
async fn mock_create_session(server: &MockServer, session_id: &str) {
    Mock::given(method("POST"))
        .and(path("/sessions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "session_id": session_id,
            })),
        )
        .expect(1..)
        .mount(server)
        .await;
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// create() calls browsr to get a session ID and stores it in the map.
#[tokio::test]
async fn test_create_stores_session() {
    let (server, sessions) = setup().await;
    mock_create_session(&server, "sess-001").await;

    let (name, _lock) = sessions.create(Some("my-browser".to_string())).await.unwrap();

    assert_eq!(name, "my-browser");
    assert_eq!(sessions.session_id_for("my-browser"), Some("sess-001".to_string()));
    assert_eq!(sessions.list().len(), 1);
}

/// create() with None name uses the session_id as the name.
#[tokio::test]
async fn test_create_with_no_name_uses_session_id() {
    let (server, sessions) = setup().await;
    mock_create_session(&server, "auto-id-42").await;

    let (name, _lock) = sessions.create(None).await.unwrap();

    assert_eq!(name, "auto-id-42");
    assert_eq!(sessions.session_id_for("auto-id-42"), Some("auto-id-42".to_string()));
}

/// create() with empty/whitespace name falls back to session_id.
#[tokio::test]
async fn test_create_with_empty_name_uses_session_id() {
    let (server, sessions) = setup().await;
    mock_create_session(&server, "fallback-id").await;

    let (name, _lock) = sessions.create(Some("  ".to_string())).await.unwrap();

    assert_eq!(name, "fallback-id");
}

/// create() with a duplicate name returns the existing session without creating a new one.
#[tokio::test]
async fn test_create_duplicate_name_returns_existing() {
    let (server, sessions) = setup().await;

    // First create needs an HTTP call
    Mock::given(method("POST"))
        .and(path("/sessions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "session_id": "original-sess",
            })),
        )
        .expect(1..=2)
        .mount(&server)
        .await;

    let (name1, _) = sessions.create(Some("dup".to_string())).await.unwrap();
    assert_eq!(name1, "dup");

    // Second create with same name — should return existing
    let (name2, _) = sessions.create(Some("dup".to_string())).await.unwrap();
    assert_eq!(name2, "dup");

    // Still only one session in the map
    assert_eq!(sessions.list().len(), 1);
    assert_eq!(sessions.session_id_for("dup"), Some("original-sess".to_string()));
}

/// ensure() with an existing session name returns it and updates last_used.
#[tokio::test]
async fn test_ensure_existing_session_reuses() {
    let (server, sessions) = setup().await;
    mock_create_session(&server, "reuse-me").await;

    // First, create a session
    sessions.create(Some("persistent".to_string())).await.unwrap();

    // ensure() with the same name should reuse it (no HTTP call)
    let (name, _lock) = sessions.ensure(Some("persistent".to_string())).await.unwrap();
    assert_eq!(name, "persistent");
    assert_eq!(sessions.list().len(), 1);
}

/// ensure() with a name that doesn't exist returns an error.
#[tokio::test]
async fn test_ensure_nonexistent_name_errors() {
    let (_server, sessions) = setup().await;

    let result = sessions.ensure(Some("ghost".to_string())).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

/// ensure() with None and no existing sessions creates a new one.
#[tokio::test]
async fn test_ensure_none_creates_when_empty() {
    let (server, sessions) = setup().await;
    mock_create_session(&server, "fresh-sess").await;

    assert!(sessions.list().is_empty());

    let (name, _lock) = sessions.ensure(None).await.unwrap();
    // When ensure(None) creates, it calls create(None) which uses session_id as name
    assert_eq!(name, "fresh-sess");
    assert_eq!(sessions.list().len(), 1);
}

/// ensure() with None and an existing session returns the first one.
#[tokio::test]
async fn test_ensure_none_reuses_first_session() {
    let (server, sessions) = setup().await;
    mock_create_session(&server, "first-sess").await;

    // Create a session first
    sessions.create(Some("existing".to_string())).await.unwrap();

    // ensure(None) should return the existing session
    let (name, _lock) = sessions.ensure(None).await.unwrap();
    assert_eq!(name, "existing");
    assert_eq!(sessions.list().len(), 1);
}

/// list() returns all session names.
#[tokio::test]
async fn test_list_returns_all_names() {
    let (server, sessions) = setup().await;

    // Create multiple sessions with different IDs
    Mock::given(method("POST"))
        .and(path("/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "session_id": "id-alpha",
        })))
        .expect(1)
        .named("create-alpha")
        .mount(&server)
        .await;

    sessions.create(Some("alpha".to_string())).await.unwrap();

    // Reset mock for second call
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "session_id": "id-beta",
        })))
        .expect(1)
        .named("create-beta")
        .mount(&server)
        .await;

    sessions.create(Some("beta".to_string())).await.unwrap();

    let mut names = sessions.list();
    names.sort();
    assert_eq!(names, vec!["alpha", "beta"]);
}

/// stop() removes the session from the map.
///
/// Note: `BrowserSessions::stop()` calls `destroy_session()` but does not
/// `.await` the future, so the HTTP DELETE never actually fires. This test
/// verifies only the in-memory cleanup.
#[tokio::test]
async fn test_stop_removes_session() {
    let (server, sessions) = setup().await;
    mock_create_session(&server, "to-stop").await;

    sessions.create(Some("doomed".to_string())).await.unwrap();
    assert_eq!(sessions.list().len(), 1);

    let removed = sessions.stop("doomed");
    assert!(removed, "stop should return true for existing session");
    assert!(sessions.list().is_empty());
    assert_eq!(sessions.session_id_for("doomed"), None);
}

/// stop() on a non-existent session returns false.
#[tokio::test]
async fn test_stop_nonexistent_returns_false() {
    let (_server, sessions) = setup().await;

    let removed = sessions.stop("nope");
    assert!(!removed, "stop should return false for non-existent session");
}

/// session_id_for() returns None for unknown names.
#[tokio::test]
async fn test_session_id_for_unknown_returns_none() {
    let (_server, sessions) = setup().await;

    assert_eq!(sessions.session_id_for("unknown"), None);
}

/// client() returns a usable BrowsrClient.
#[tokio::test]
async fn test_client_accessor() {
    let (server, sessions) = setup().await;
    let client = sessions.client();
    // The client should be pointed at our mock server
    assert_eq!(client.base_url(), server.uri());
}
