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
use ecsdk::prelude::IsomorphicApp;
use render::RenderMode;
use tokio::net::UnixStream;

use crate::client::run_client;
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

pub fn create_isomorphic_app() -> IsomorphicApp<Message, Command> {
    let mut iso = IsomorphicApp::new();
    iso.add_plugin(SharedReplicationPlugin);
    iso.add_plugin(ConnectionPlugin);
    iso.add_scoped_plugin(StatusFeature);
    iso
}

struct ClientLaunch {
    mode: RenderMode,
    command: Command,
}

enum Launch {
    Daemon,
    Client(ClientLaunch),
}

impl Cli {
    fn into_launch(self) -> Launch {
        if self.daemon {
            Launch::Daemon
        } else {
            Launch::Client(ClientLaunch {
                mode: resolve_render_mode(self.output),
                command: self.command.unwrap_or(Command::Up),
            })
        }
    }
}

impl ClientLaunch {
    async fn prepare(self) -> Self {
        match self.command {
            Command::Up => {
                if !daemon_is_running().await {
                    spawn_daemon().await;
                }
            }
            Command::Status => {
                if !daemon_is_running().await {
                    eprintln!("compose status requires a running daemon. Start it with `compose up`.");
                    std::process::exit(1);
                }
            }
        }

        self
    }

    async fn run(self) {
        run_client(create_isomorphic_app(), self.mode, self.command).await;
    }
}

impl Launch {
    async fn run(self) {
        match self {
            Launch::Daemon => run_daemon(create_isomorphic_app()).await,
            Launch::Client(client) => client.prepare().await.run().await,
        }
    }
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

#[tokio::main]
async fn main() {
    Cli::parse().into_launch().run().await;
}
