use std::io;
use std::path::{Path, PathBuf};

use roam::Tx;
use roam_stream::Connector;
use tokio::net::UnixStream;

use crate::protocol::DaemonEvent;

pub const SOCKET_PATH: &str = "/tmp/ecs-compose-daemon.sock";

#[roam::service]
pub trait ComposeDaemon {
    /// Subscribe to real-time events. Daemon sends snapshot first, then deltas.
    async fn subscribe(&self, events: Tx<DaemonEvent>);

    /// Request graceful shutdown of all containers.
    async fn shutdown(&self) -> Result<String, String>;

    /// Health check — verifies the daemon is alive.
    async fn ping(&self) -> Result<String, String>;
}

/// Connector for client-side Unix socket connections to the daemon.
#[derive(Clone)]
pub struct DaemonConnector {
    pub path: PathBuf,
}

impl DaemonConnector {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl Connector for DaemonConnector {
    type Transport = UnixStream;

    async fn connect(&self) -> io::Result<UnixStream> {
        UnixStream::connect(&self.path).await
    }
}
