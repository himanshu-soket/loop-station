//! The embedded dashboard (`/dashboard`) serves traces, system metrics and
//! the inference console from inside the running `loop` process, sharing the
//! live runtime's engine. These tests boot a real CLI runtime against an
//! isolated agent dir and verify the served routes.

use loop_cli::config::paths::{settings_path, ENV_AGENT_DIR};
use loop_cli::{bootstrap_cli, dashboard, BootstrapOpts};

async fn boot(cwd: &std::path::Path) -> loop_cli::CliRuntime {
    bootstrap_cli(BootstrapOpts {
        cwd: cwd.to_path_buf(),
        provider: None,
        model: None,
        theme: None,
        system_prompt: None,
        append_system_prompt: None,
        no_context_files: true,
        no_web: false,
        permission_mode: None,
        interactive: false,
        session_id: None,
        agent: None,
    })
    .await
    .unwrap()
}

/// Isolated agent dir so the tests never touch `~/.loop`.
///
/// `ENV_AGENT_DIR` is process-global, so tests that set it must hold this
/// lock for their entire body (including awaits).
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn isolate() -> (tempfile::TempDir, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    // Mirror the real layout: <root>/.loop/agent, so the console store lands
    // at <root>/.loop/dashboard/store.db just like ~/.loop/dashboard.
    let agent_dir = root.path().join(".loop").join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let cwd = tempfile::tempdir().unwrap();
    // A saved keyless custom provider keeps bootstrap happy without secrets.
    std::fs::write(
        settings_path(&agent_dir),
        r#"{"providers":[{"id":"local","name":"Local","baseUrl":"http://127.0.0.1:1/v1"}]}"#,
    )
    .unwrap();
    std::env::set_var(ENV_AGENT_DIR, &agent_dir);
    (root, cwd)
}

/// Boot the dashboard and authenticate as the owner, returning a client
/// preloaded with the session cookie. Sign-up is only possible on a fresh
/// auth DB (single-owner model), which every isolated test run is.
///
/// reqwest here has no cookie-store feature; the session cookie is captured
/// from the sign-in response and attached explicitly — mirroring what a
/// browser does automatically.
async fn authenticated_client(base: &str) -> (reqwest::Client, String) {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    // Create the owner account (first run in the isolated agent dir).
    let res = client
        .post(format!("{base}/api/auth/signup"))
        .json(&serde_json::json!({
            "identifier": "test-owner",
            "password": "correct horse battery"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 201, "sign-up must succeed on first run");
    // Sign in — capture the session cookie the browser would store.
    let res = client
        .post(format!("{base}/api/auth/signin"))
        .json(&serde_json::json!({
            "identifier": "test-owner",
            "password": "correct horse battery"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "sign-in must succeed");
    let set_cookie = res
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .expect("sign-in must set a session cookie")
        .to_string();
    let token = set_cookie.split(';').next().unwrap().to_string();
    (client, token)
}


/// GET a path with the session cookie attached (browser-equivalent).
async fn authed_get(
    client: &reqwest::Client,
    base: &str,
    cookie: &str,
    path: &str,
) -> reqwest::Response {
    client
        .get(format!("{base}{path}"))
        .header("cookie", cookie)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn embedded_dashboard_serves_all_surfaces() {
    let _env = ENV_LOCK.lock().await;
    let (_root, cwd) = isolate();
    let runtime = boot(cwd.path()).await;
    let state = std::sync::Arc::new(dashboard::DashboardState::default());

    let handle = dashboard::start(&runtime, &state).await.unwrap();
    assert!(handle.port > 0);

    let base = format!("http://127.0.0.1:{}", handle.port);
    // Unauthenticated requests are redirected to sign-in (deep link kept).
    {
        let res = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .get(format!("{base}/console"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 303);
        let loc = res.headers().get("location").unwrap().to_str().unwrap();
        assert!(loc.starts_with("/signin?next="), "deep link preserved: {loc}");
    }
    let (client, session_cookie) = authenticated_client(&base).await;

    // Starting again reports the same dashboard (no second server).
    let again = dashboard::start(&runtime, &state).await.unwrap();
    assert_eq!(again.port, handle.port);

    // Traces observatory + system monitor + console pages.
    for path in ["/", "/system", "/console"] {
        let res = authed_get(&client, &base, &session_cookie, path).await;
        assert_eq!(res.status(), 200, "GET {path}");
        assert!(res.text().await.unwrap().len() > 100);
    }

    // Each surface serves its own shell with the shared portal branding:
    // /console is the welcome/workspace page first (not a dashboard), and
    // the other two keep their dedicated observability/system identities.
    {
        let console_page = authed_get(&client, &base, &session_cookie, "/console")
            .await
            .text()
            .await
            .unwrap();
        assert!(
            console_page.contains("Loop · AI Console"),
            "/console must serve the Loop AI Console shell"
        );
        assert!(
            console_page.contains("/console/console-app.js"),
            "/console must load the console SPA script"
        );

        let root_page = authed_get(&client, &base, &session_cookie, "/")
            .await
            .text()
            .await
            .unwrap();
        assert!(
            root_page.contains("Loop · Observability"),
            "/ must serve the Loop Observability shell"
        );

        let system_page = authed_get(&client, &base, &session_cookie, "/system")
            .await
            .text()
            .await
            .unwrap();
        assert!(
            system_page.contains("Loop · System Dashboard"),
            "/system must serve the System Dashboard shell"
        );
    }

    // The console SPA carries the portal contract: the welcome view with
    // both workspace cards, plus the chat composer and navigation entries.
    {
        let spa = authed_get(&client, &base, &session_cookie, "/console/console-app.js")
            .await
            .text()
            .await
            .unwrap();
        assert!(
            spa.contains("Welcome to Loop"),
            "SPA must render the welcome view"
        );
        assert!(
            spa.contains("'/system#overview'") && spa.contains("title: 'Loop AI Console'"),
            "welcome view must link to the system workspace and the AI console"
        );
        assert!(
            spa.contains("id=\"chatInput\""),
            "SPA must contain the chat composer"
        );
        assert!(
            spa.contains("'Playground'") && spa.contains("'Traces & Requests'"),
            "SPA navigation must expose chat and traces views"
        );
    }

    // Console APIs answer with honest empty state.
    let models: serde_json::Value = authed_get(&client, &base, &session_cookie, "/api/models")
        .await
        .json()
        .await
        .unwrap();
    assert!(models["models"].is_array());

    let convos: serde_json::Value = authed_get(&client, &base, &session_cookie, "/api/conversations")
        .await
        .json()
        .await
        .unwrap();
    assert!(convos["conversations"].is_array());

    let traces: serde_json::Value = authed_get(&client, &base, &session_cookie, "/api/traces")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(traces["total"], 0);

    let analytics: serde_json::Value = authed_get(&client, &base, &session_cookie, "/api/analytics")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(analytics["totalRequests"], 0);

    // System monitor is sampling.
    let overview: serde_json::Value = authed_get(&client, &base, &session_cookie, "/api/sys/overview")
        .await
        .json()
        .await
        .unwrap();
    assert!(overview.is_object());

    // The console store lives under the isolated agent dir, not ~/.loop.
    let db = loop_dashboard::engine::EnginePaths::resolve()
        .agent_dir
        .parent()
        .unwrap()
        .join("dashboard")
        .join("store.db");
    assert!(db.exists(), "console store at {}", db.display());
    assert!(db.starts_with(_root.path()));

    // Stop frees the port and reports it.
    let stopped_port = state.stop().unwrap();
    assert_eq!(stopped_port, handle.port);
    assert!(state.stop().is_none());
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let res = client.get(format!("{base}/api/models")).send().await;
    assert!(res.is_err(), "listener must be gone after stop");
}

#[tokio::test]
async fn dashboard_shares_the_live_engine() {
    let _env = ENV_LOCK.lock().await;
    let (_root, cwd) = isolate();
    let runtime = boot(cwd.path()).await;

    // The embedded engine must reflect the LIVE Models collection — a model
    // registered on the runtime appears through the console API.
    let models_arc = runtime.models.clone();
    models_arc.set_provider(loop_ai::providers::custom_provider(
        loop_ai::providers::CustomProviderConfig {
            id: "live-proof".into(),
            name: Some("Live Proof".into()),
            base_url: "http://127.0.0.1:1/v1".into(),
            api_key_env: vec![],
            models: vec![loop_ai::providers::CustomModelSpec::new("live-model")],
            headers: None,
        },
    ));

    let state = std::sync::Arc::new(dashboard::DashboardState::default());
    let handle = dashboard::start(&runtime, &state).await.unwrap();
    let base = format!("http://127.0.0.1:{}", handle.port);
    let (client, session_cookie) = authenticated_client(&base).await;

    let models: serde_json::Value = authed_get(&client, &base, &session_cookie, "/api/models")
        .await
        .json()
        .await
        .unwrap();
    let ids: Vec<String> = models["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        ids.contains(&"live-model".to_string()),
        "live-registered model must appear, got {ids:?}"
    );

    state.stop();
}
