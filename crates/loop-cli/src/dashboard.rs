//! Embedded dashboard + inference console, served by the `loop` process.
//!
//! `/dashboard [on|off|status]` starts a localhost-only axum server inside
//! the running Loop CLI. It reuses the live application runtime — the same
//! `Models` collection and credential store the agent prompts through — so
//! the console's playground streams through the real engine rather than a
//! re-read copy of its configuration. Traces, requests and conversations
//! persist in the dashboard's own SQLite store under `~/.loop/dashboard/`.

use std::sync::Arc;

use loop_app_core::Runtime;
use tokio_util::sync::CancellationToken;

/// Default localhost port for the embedded dashboard.
pub const DEFAULT_PORT: u16 = 4173;

/// Handle to a running embedded dashboard.
#[derive(Clone)]
pub struct DashboardHandle {
    /// Port the dashboard is bound to.
    pub port: u16,
    /// Cancels the serving task (and with it the listener) when dropped or
    /// triggered by `/dashboard off`.
    pub cancel: CancellationToken,
}

/// State shared by the `/dashboard` command across TUI redraws.
#[derive(Default)]
pub struct DashboardState {
    inner: parking_lot::Mutex<Option<DashboardHandle>>,
}

impl DashboardState {
    /// The running handle, if the dashboard was started.
    pub fn get(&self) -> Option<DashboardHandle> {
        self.inner.lock().clone()
    }

    /// Stop the dashboard, returning the port it was on.
    pub fn stop(&self) -> Option<u16> {
        let handle = self.inner.lock().take()?;
        handle.cancel.cancel();
        Some(handle.port)
    }

    /// Remember a freshly started dashboard.
    fn set(&self, handle: DashboardHandle) {
        *self.inner.lock() = Some(handle);
    }
}

/// Where the console's durable store lives.
fn console_db_path() -> anyhow::Result<std::path::PathBuf> {
    Ok(loop_dashboard::engine::EnginePaths::resolve()
        .agent_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot resolve ~/.loop"))?
        .join("dashboard")
        .join("store.db"))
}

/// Where the authentication store lives (its own DB, beside the console
/// store, so the trace/conversation schema stays untouched).
fn auth_db_path() -> anyhow::Result<std::path::PathBuf> {
    Ok(console_db_path()?.with_file_name("auth.db"))
}

/// Start (or report) the embedded dashboard.
///
/// The router is the exact one the standalone `loop-dashboard` binary serves:
/// traces observatory (`/`), system monitor (`/system`) and the inference
/// console (`/console`), wired to this runtime's live engine.
pub async fn start(runtime: &Runtime, state: &DashboardState) -> anyhow::Result<DashboardHandle> {
    if let Some(existing) = state.get() {
        return Ok(existing);
    }

    let db_path = console_db_path()?;
    let store = Arc::new(loop_dashboard::persist::DashboardStore::open(&db_path)?);

    // Share the LIVE engine: the console sees every provider and model the
    // running app has, including logins made after startup.
    let engine = Arc::new(loop_dashboard::engine::Engine::from_parts(
        Arc::clone(&runtime.models),
        Arc::clone(&runtime.credentials) as Arc<dyn loop_ai::CredentialStore>,
    ));
    let playground = Arc::new(loop_dashboard::playground::Playground::new(
        Arc::clone(&engine),
        Arc::clone(&store),
    ));

    // Cookie-session authentication over the same surfaces. The auth
    // DB is separate from the console store.
    let auth_store = Arc::new(loop_dashboard::auth::AuthStore::open(auth_db_path()?)?);
    // Help & Support tickets: own DB, mounted with the auth surface.
    let support_store = Arc::new(loop_dashboard::support::store::SupportStore::open(
        console_db_path()?.with_file_name("support.db"),
    )?);
    let (app, monitor) = loop_dashboard::api::build_app(loop_dashboard::api::AppOptions {
        demo: false,
        db: Some(runtime.sessions_db.clone()),
        sys_demo: false,
        sys_interval: std::time::Duration::from_millis(2000),
        console_state: Some(Arc::new(loop_dashboard::console::ConsoleState {
            engine,
            store,
            playground,
            auth: Some(Arc::new(loop_dashboard::auth_http::AuthState::new(
                auth_store.clone(),
            ))),
        })),
        auth_store: Some(auth_store),
        support_store: Some(support_store),
    })?;

    let cancel = CancellationToken::new();

    // Bind before reporting success so the caller can open a working URL.
    let listener = {
        let cancel = cancel.clone();
        tokio::select! {
            biased;
            _ = cancel.cancelled() => anyhow::bail!("dashboard cancelled before start"),
            res = tokio::net::TcpListener::bind(("127.0.0.1", 0)) => res?,
        }
    };
    let port = listener
        .local_addr()
        .map(|a| a.port())
        .unwrap_or(DEFAULT_PORT);

    // Publish the ephemeral port so the Zsh terminal hook (installed by
    // `loop install-terminal`) can find this dashboard instance.
    if let Err(e) = loop_dashboard::termhook::EndpointConfig::set_port(port) {
        tracing::warn!(error = %e, "could not publish dashboard port for terminal hooks");
    }

    // System-monitor sampling loop (real metrics, default cadence).
    {
        let monitor = monitor.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => (),
                _ = loop_dashboard::sysmon::run_sampler(monitor, false, std::time::Duration::from_millis(2000)) => (),
            }
        });
    }

    // Serve until cancelled. The listener is dropped when this task ends,
    // which frees the port immediately.
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let serve = axum::serve(listener, app);
            tokio::select! {
                biased;
                _ = cancel.cancelled() => (),
                res = serve => {
                    if let Err(err) = res {
                        tracing::warn!(error = %err, "embedded dashboard stopped");
                    }
                },
            }
        });
    }

    let handle = DashboardHandle { port, cancel };
    state.set(handle.clone());
    Ok(handle)
}
