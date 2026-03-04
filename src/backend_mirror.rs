use std::sync::Arc;

use tokio::sync::mpsc;

use crate::backend::{ContainerBackend, PullProgress};
use crate::protocol::{DaemonEvent, Phase};

/// A backend that replays daemon events instead of doing real work.
/// Each instance owns a dedicated mpsc receiver — the client forwarder
/// demuxes by container ID and sends only relevant events here.
#[derive(Clone)]
pub struct MirrorBackend {
    rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<DaemonEvent>>>,
}

impl MirrorBackend {
    pub fn new(rx: mpsc::UnboundedReceiver<DaemonEvent>) -> Self {
        Self {
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
        }
    }
}

impl ContainerBackend for MirrorBackend {
    async fn pull_image(
        &self,
        progress_tx: mpsc::UnboundedSender<PullProgress>,
        log_tx: mpsc::UnboundedSender<String>,
    ) -> Result<(), String> {
        let mut rx = self.rx.lock().await;
        loop {
            match rx.recv().await {
                Some(DaemonEvent::Progress {
                    downloaded, total, ..
                }) => {
                    let _ = progress_tx.send(PullProgress { downloaded, total });
                }
                Some(DaemonEvent::Log { text, .. }) => {
                    let _ = log_tx.send(text);
                }
                Some(DaemonEvent::PhaseChanged { phase, .. }) => {
                    if matches!(phase, Phase::Starting | Phase::Running | Phase::Stopped) {
                        return Ok(());
                    }
                }
                None => return Err("disconnected".into()),
                _ => {}
            }
        }
    }

    async fn boot_container(
        &self,
        log_tx: mpsc::UnboundedSender<String>,
    ) -> Result<(), String> {
        let mut rx = self.rx.lock().await;
        loop {
            match rx.recv().await {
                Some(DaemonEvent::Log { text, .. }) => {
                    let _ = log_tx.send(text);
                }
                Some(DaemonEvent::PhaseChanged { phase, .. }) => {
                    if matches!(phase, Phase::Running | Phase::Stopped) {
                        return Ok(());
                    }
                }
                None => return Err("disconnected".into()),
                _ => {}
            }
        }
    }

    async fn stop_container(&self) -> Result<(), String> {
        let mut rx = self.rx.lock().await;
        loop {
            match rx.recv().await {
                Some(DaemonEvent::PhaseChanged { phase, .. }) => {
                    if phase == Phase::Stopped {
                        return Ok(());
                    }
                }
                None => return Err("disconnected".into()),
                _ => {}
            }
        }
    }
}
