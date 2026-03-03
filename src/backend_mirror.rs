use std::collections::HashMap;

use tokio::sync::{broadcast, mpsc};

use crate::backend::{ContainerBackend, PullProgress};
use crate::protocol::{DaemonEvent, Phase};

/// A backend that replays daemon events instead of doing real work.
/// Each method subscribes to the broadcast, filters for its container,
/// forwards progress/log events through the standard channels, and returns
/// when the daemon signals the phase has advanced.
#[derive(Clone)]
pub struct MirrorBackend {
    pub events: broadcast::Sender<DaemonEvent>,
    pub image_to_name: HashMap<String, String>,
}

impl ContainerBackend for MirrorBackend {
    async fn pull_image(
        &self,
        image: &str,
        progress_tx: mpsc::UnboundedSender<PullProgress>,
        log_tx: mpsc::UnboundedSender<String>,
    ) -> Result<(), String> {
        let name = self
            .image_to_name
            .get(image)
            .ok_or_else(|| format!("unknown image: {image}"))?
            .clone();
        let mut rx = self.events.subscribe();
        loop {
            match rx.recv().await {
                Ok(DaemonEvent::Progress {
                    id,
                    downloaded,
                    total,
                }) if id == name => {
                    let _ = progress_tx.send(PullProgress { downloaded, total });
                }
                Ok(DaemonEvent::Log { id, text }) if id == name => {
                    let _ = log_tx.send(text);
                }
                Ok(DaemonEvent::PhaseChanged { id, phase }) if id == name => {
                    if matches!(phase, Phase::Starting | Phase::Running | Phase::Stopped) {
                        return Ok(());
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err("disconnected".into());
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                _ => {}
            }
        }
    }

    async fn boot_container(
        &self,
        name: &str,
        log_tx: mpsc::UnboundedSender<String>,
    ) -> Result<(), String> {
        let name = name.to_string();
        let mut rx = self.events.subscribe();
        loop {
            match rx.recv().await {
                Ok(DaemonEvent::Log { id, text }) if id == name => {
                    let _ = log_tx.send(text);
                }
                Ok(DaemonEvent::PhaseChanged { id, phase }) if id == name => {
                    if matches!(phase, Phase::Running | Phase::Stopped) {
                        return Ok(());
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err("disconnected".into());
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                _ => {}
            }
        }
    }

    async fn stop_container(&self, name: &str) -> Result<(), String> {
        let name = name.to_string();
        let mut rx = self.events.subscribe();
        loop {
            match rx.recv().await {
                Ok(DaemonEvent::PhaseChanged { id, phase }) if id == name => {
                    if phase == Phase::Stopped {
                        return Ok(());
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err("disconnected".into());
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                _ => {}
            }
        }
    }
}
