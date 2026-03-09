use crate::backend_mock::MockBackend;

pub struct PullProgress {
    pub downloaded: u64,
    pub total: u64,
}

#[allow(async_fn_in_trait)]
pub trait ContainerBackend {
    /// Pull an image. Calls `on_progress` with download progress and `on_log` with log lines.
    async fn pull_image(
        &self,
        on_progress: impl Fn(PullProgress) + Send,
        on_log: impl Fn(String) + Send,
    ) -> anyhow::Result<()>;

    /// Boot a container. Calls `on_log` with log lines as it starts up.
    async fn boot_container(&self, on_log: impl Fn(String) + Send) -> anyhow::Result<()>;

    /// Stop a container.
    async fn stop_container(&self) -> anyhow::Result<()>;
}

pub type ContainerRuntime = MockBackend;
