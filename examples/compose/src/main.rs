mod backend;
mod backend_mock;
mod client;
mod container;
mod daemon;
mod ipc;
mod lifecycle;
mod message;
mod protocol;
mod render;
#[cfg(test)]
mod replay;
mod replicon;
mod status;

use std::io::IsTerminal;
use std::time::Duration;

use clap::{Parser, Subcommand};
use ecsdk::prelude::{IsomorphicApp, IsomorphicPlugin, WakeSignal};
use render::RenderMode;
use tokio::net::UnixStream;
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::client::{run_status, run_up};
use crate::daemon::run_daemon;
use crate::ipc::SOCKET_PATH;
use crate::message::Message;
use crate::replicon::{ConnectionPlugin, SharedReplicationPlugin};
use crate::status::StatusFeature;

#[derive(Parser)]
#[command(about = "ECS-driven container orchestration demo")]
struct Cli {
    /// Output mode (plain or tui). Defaults to tui when stdout is a terminal.
    #[arg(long, value_enum)]
    output: Option<RenderMode>,

    /// Run in daemon mode (no UI, serves IPC).
    #[arg(short, long)]
    daemon: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Up,
    Status,
}

pub fn create_isomorphic_app() -> IsomorphicApp<Message> {
    let mut iso = IsomorphicApp::new();
    iso.add_plugin(SharedReplicationPlugin);
    iso.add_plugin(ConnectionPlugin);
    iso.add_plugin(StatusFeature);
    iso.add_plugin(TracingPlugin);
    iso
}

fn resolve_render_mode(explicit: Option<RenderMode>) -> RenderMode {
    explicit.unwrap_or_else(|| {
        if std::io::stdout().is_terminal() {
            RenderMode::Tui
        } else {
            RenderMode::Plain
        }
    })
}

/// Check if a daemon is already listening on the socket.
async fn daemon_is_running() -> bool {
    UnixStream::connect(SOCKET_PATH).await.is_ok()
}

/// Spawn the daemon as a background process and wait until it's ready.
async fn spawn_daemon() {
    let exe = std::env::current_exe().expect("Failed to get current executable path");
    std::process::Command::new(exe)
        .arg("--daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .expect("Failed to spawn daemon");

    // Wait for daemon to be ready
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if daemon_is_running().await {
            return;
        }
    }
    eprintln!("Timed out waiting for daemon to start");
    std::process::exit(1);
}

async fn ensure_daemon(auto_spawn: bool) {
    if daemon_is_running().await {
        return;
    }

    if auto_spawn {
        spawn_daemon().await;
    } else {
        eprintln!("compose commands requires a running daemon. Start it with `compose up`.");
        std::process::exit(1);
    }
}

struct TracingPlugin;

impl IsomorphicPlugin for TracingPlugin {
    fn build_shared(&self, app: &mut ecsdk::prelude::App) {
        let wake = app.world().resource::<WakeSignal>().clone();
        let (tracing_layer, tracing_receiver) = ecsdk::tracing::setup(wake);
        tracing_subscriber::registry()
            .with(
                tracing_layer.with_filter(
                    tracing_subscriber::filter::Targets::new()
                        .with_target("compose", tracing::Level::INFO),
                ),
            )
            .init();
        app.add_plugins(ecsdk::tracing::TracingPlugin::new(tracing_receiver));
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let iso = create_isomorphic_app();

    if cli.daemon {
        let app = iso.build_server();
        let app = run_daemon(app);
        app.run().await;
        let _ = std::fs::remove_file(crate::ipc::SOCKET_PATH);
        tracing::info!("Daemon shut down");
        return;
    }

    let command = cli.command.unwrap_or(Command::Up);
    ensure_daemon(matches!(command, Command::Up)).await;

    let app = iso.build_client();
    let app = match command {
        Command::Up => run_up(app, resolve_render_mode(cli.output)),
        Command::Status => run_status(app),
    };

    app.run().await;
}
