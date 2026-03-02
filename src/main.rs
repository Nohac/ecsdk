use std::io::IsTerminal;
use std::time::Duration;

use clap::Parser;
use tokio::net::UnixStream;

use ecstest::client::run_client;
use ecstest::daemon::run_daemon;
use ecstest::ipc::SOCKET_PATH;
use ecstest::render::RenderMode;

#[derive(Parser)]
#[command(about = "ECS-driven container orchestration demo")]
struct Cli {
    /// Output mode (plain or tui). Defaults to tui when stdout is a terminal.
    #[arg(long, value_enum)]
    output: Option<RenderMode>,

    /// Run in daemon mode (no UI, serves IPC).
    #[arg(short, long)]
    daemon: bool,
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
    let cli = Cli::parse();

    if cli.daemon {
        run_daemon().await;
    } else {
        let mode = resolve_render_mode(cli.output);

        // Auto-spawn daemon if not already running
        if !daemon_is_running().await {
            spawn_daemon().await;
        }

        run_client(mode).await;
    }
}
