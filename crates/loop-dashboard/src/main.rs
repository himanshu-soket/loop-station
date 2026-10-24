//! loop-dashboard — local read-only observability dashboard for Loop traces.
//!
//! Serves a single-page dashboard over the session database at
//! `~/.loop/agent/sessions.db`. Binds to 127.0.0.1 only; trace content never
//! leaves localhost.

mod analytics;
mod api;
mod demo;
mod payload;
mod store;
mod sysmon;
mod web;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use clap::Parser;

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
    let (app, monitor) = api::build_app(api::AppOptions {
        demo: cli.demo,
        db: cli.db.as_deref().map(|p| p.to_path_buf()),
        sys_demo: cli.sys_demo,
        sys_interval: interval,
    })?;

    // System-monitor sampling loop: real or clearly-labeled synthetic data.
    tokio::spawn(sysmon::run_sampler(monitor, cli.sys_demo, interval));

    let listener = tokio::net::TcpListener::bind(addr).await?;
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
    axum::serve(listener, app).await?;
    Ok(())
}
