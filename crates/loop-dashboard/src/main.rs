//! loop-dashboard — local read-only observability dashboard for Loop traces.
//!
//! Serves a single-page dashboard over the session database at
//! `~/.loop/agent/sessions.db`. Binds to 127.0.0.1 only; trace content never
//! leaves localhost.
//!
//! The same router (including the inference console) is embedded in the `loop`
//! CLI itself; this binary remains for standalone use.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use clap::Parser;
use loop_dashboard::{api, auth, auth_http, console, engine, persist, playground, sysmon};

/// Dashboard server options.
#[derive(Debug, Parser)]
#[command(
    name = "loop-dashboard",
    about = "Local read-only observability dashboard for Loop session traces"
)]
struct Cli {
    /// Bind address (forced to loopback unless --allow-lan is passed).
    #[arg(long, default_value = "127.0.0.1")]
    host: IpAddr,

    /// Port to listen on.
    #[arg(long, default_value = "4173")]
    port: u16,

    /// Path to the sessions database (default: ~/.loop/agent/sessions.db).
    #[arg(long)]
    db: Option<PathBuf>,

    /// Serve clearly-labeled synthetic data instead of real traces.
    /// Never mixes with real data.
    #[arg(long)]
    demo: bool,

    /// System-monitor sampling interval in milliseconds (500–60000).
    #[arg(long, default_value_t = 2000)]
    sys_interval_ms: u64,

    /// Serve clearly-labeled synthetic system metrics instead of real ones.
    #[arg(long)]
    sys_demo: bool,

    /// Allow binding a non-loopback address (not recommended).
    #[arg(long)]
    allow_lan: bool,

    /// Path to the dashboard's own durable store (traces, requests,
    /// conversations). Default: ~/.loop/dashboard/store.db.
    #[arg(long)]
    console_db: Option<PathBuf>,

    /// Disable authentication entirely (historical open-access mode).
    /// Intended for scripted local use; not recommended otherwise.
    #[arg(long)]
    no_auth: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if !cli.host.is_loopback() && !cli.allow_lan {
        anyhow::bail!(
            "refusing to bind {}: pass --allow-lan to expose the dashboard beyond localhost",
            cli.host
        );
    }

    let addr = SocketAddr::new(cli.host, cli.port);
    let interval = std::time::Duration::from_millis(cli.sys_interval_ms.clamp(500, 60_000));

    // Durable console store (traces / requests / conversations).
    let console_db = match cli.console_db.clone() {
        Some(p) => p,
        None => dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("$HOME not set; pass --console-db explicitly"))?
            .join(".loop")
            .join("dashboard")
            .join("store.db"),
    };
    let console_store = std::sync::Arc::new(persist::DashboardStore::open(&console_db)?);

    // Real inference engine, wired to Loop's on-disk configuration.
    let engine = std::sync::Arc::new(engine::Engine::load(engine::EnginePaths::resolve()));
    let playground = std::sync::Arc::new(playground::Playground::new(
        std::sync::Arc::clone(&engine),
        std::sync::Arc::clone(&console_store),
    ));
    let console_store_path = console_store.path().display().to_string();

    // Authentication store: its own database beside the console store, so
    // the trace/conversation schema stays untouched. `--no-auth` keeps the
    // historical open-access behavior for scripted/local use.
    let auth_store = if cli.no_auth {
        None
    } else {
        let auth_db = console_db.with_file_name("auth.db");
        Some(std::sync::Arc::new(auth::AuthStore::open(&auth_db)?))
    };

    let (app, monitor) = api::build_app(api::AppOptions {
        demo: cli.demo,
        db: cli.db.as_deref().map(|p| p.to_path_buf()),
        sys_demo: cli.sys_demo,
        sys_interval: interval,
        console_state: Some(std::sync::Arc::new(console::ConsoleState {
            engine,
            store: console_store,
            playground,
            auth: auth_store
                .as_ref()
                .map(|a| std::sync::Arc::new(auth_http::AuthState { store: a.clone() })),
        })),
        auth_store,
    })?;

    // System-monitor sampling loop: real or clearly-labeled synthetic data.
    tokio::spawn(sysmon::run_sampler(monitor, cli.sys_demo, interval));

    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Publish the bound port for the Zsh terminal hook (which reads it from
    // ~/.loop/dashboard/loop.json on every report).
    if let Err(e) =
        loop_dashboard::termhook::EndpointConfig::set_port(listener.local_addr()?.port())
    {
        eprintln!("warning: could not publish dashboard port for terminal hooks: {e}");
    }
    println!(
        "loop-dashboard {} → http://{addr}",
        if cli.demo || cli.sys_demo {
            "[DEMO DATA]"
        } else {
            ""
        }
    );
    println!(
        "  traces : http://{addr}/  ({})",
        if cli.demo {
            "DEMO"
        } else {
            "read-only, localhost-bound"
        }
    );
    println!(
        "  system : http://{addr}/system  ({})",
        if cli.sys_demo {
            "DEMO — synthetic metrics"
        } else {
            "live local metrics"
        }
    );
    println!("  sampling every {}ms", interval.as_millis());
    println!("  console: http://{addr}/console  (chat playground + traces + requests)");
    println!("  store  : {console_store_path}");
    axum::serve(listener, app).await?;
    Ok(())
}
