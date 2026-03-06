use std::io;
use std::path::{Path, PathBuf};

use roam_stream::Connector;
use tokio::net::UnixStream;

pub const SOCKET_PATH: &str = "/tmp/ecs-compose-daemon.sock";

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
