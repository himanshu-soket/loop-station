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
    let app = api::build_app(cli.demo, cli.db.as_deref().map(|p| p.to_path_buf()))?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!(
        "loop-dashboard {} → http://{addr}",
        if cli.demo { "[DEMO DATA]" } else { "" }
    );
    if cli.demo {
        println!("  mode: DEMO (synthetic data, clearly labeled in the UI)");
    } else {
        println!("  mode: read-only, localhost-bound");
    }
    axum::serve(listener, app).await?;
    Ok(())
}
