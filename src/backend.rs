use tokio::sync::mpsc;

use crate::backend_mock::MockBackend;

pub struct PullProgress {
    pub downloaded: u64,
    pub total: u64,
}

#[allow(async_fn_in_trait)]
pub trait ContainerBackend {
    /// Pull an image. Sends progress updates via `progress_tx` and log lines via `log_tx`.
    async fn pull_image(
        &self,
        progress_tx: mpsc::UnboundedSender<PullProgress>,
        log_tx: mpsc::UnboundedSender<String>,
    ) -> Result<(), String>;

    /// Boot a container. Sends log lines as it starts up.
    async fn boot_container(&self, log_tx: mpsc::UnboundedSender<String>) -> Result<(), String>;

    /// Stop a container.
    async fn stop_container(&self) -> Result<(), String>;
}

/// Only MockBackend remains — MirrorBackend is replaced by replicon replication.
pub type ContainerRuntime = MockBackend;
