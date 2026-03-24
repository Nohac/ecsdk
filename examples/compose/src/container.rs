use ecsdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Component, Serialize, Deserialize)]
pub struct ContainerName(pub String);

#[derive(Component, Serialize, Deserialize)]
pub struct ImageRef(pub String);

/// Lower starts first; containers with the same order start in parallel.
#[derive(Component, Serialize, Deserialize)]
pub struct StartOrder(pub u32);

#[derive(Component, StateComponent, PartialEq, Eq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ContainerPhase {
    Pending,
    PullingImage,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
}

impl ContainerPhase {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::PullingImage => "Pulling image",
            Self::Starting => "Starting",
            Self::Running => "Running",
            Self::Stopping => "Stopping",
            Self::Stopped => "Stopped",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Component, StateComponent, PartialEq, Eq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum OrchestratorPhase {
    Deploying,
    AllRunning,
    ShuttingDown,
    AllStopped,
}

#[derive(Component, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub text: String,
}

#[derive(Component, Default)]
pub struct LogBuffer {
    pub lines: Vec<LogLine>,
}

impl LogBuffer {
    pub fn push(&mut self, text: impl Into<String>) {
        self.lines.push(LogLine { text: text.into() });
    }
}

#[derive(Component, Serialize, Deserialize)]
#[relationship(relationship_target = LogView)]
#[component(immutable)]
pub struct LogEntry {
    #[entities]
    #[relationship]
    pub target: Entity,
    pub sequence: u64,
    pub label: String,
    pub color_idx: u8,
    pub message: String,
}

#[derive(Component, Default, Serialize, Deserialize)]
#[relationship_target(relationship = LogEntry, linked_spawn)]
pub struct LogView(Vec<Entity>);

/// Marker for the system-wide log entity (global messages).
#[derive(Component, Serialize, Deserialize)]
pub struct SystemEntity;

/// Drains tracing events into entity LogBuffers.
pub fn drain_tracing_logs(
    mut receiver: ResMut<ecsdk::tracing::TracingReceiver>,
    mut logs: Query<&mut LogBuffer>,
    system_entity: Query<Entity, With<SystemEntity>>,
) {
    while let Ok(event) = receiver.rx.try_recv() {
        let target = event.entity.or_else(|| system_entity.single().ok());
        let Some(target) = target else {
            continue;
        };
        if let Ok(mut log_buf) = logs.get_mut(target) {
            log_buf.push(event.message);
        }
    }
}
