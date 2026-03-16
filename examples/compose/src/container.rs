use std::collections::HashMap;
use std::time::Instant;

use bevy::ecs::prelude::*;
use ecsdk_macros::StateComponent;
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
    pub timestamp: Instant,
    pub text: String,
}

#[derive(Component, Default)]
pub struct LogBuffer {
    pub lines: Vec<LogLine>,
}

impl LogBuffer {
    pub fn push(&mut self, text: impl Into<String>) {
        self.lines.push(LogLine {
            timestamp: Instant::now(),
            text: text.into(),
        });
    }
}

/// Marker for the system-wide log entity (global messages).
#[derive(Component, Serialize, Deserialize)]
pub struct SystemEntity;

#[derive(Component, Serialize, Deserialize)]
pub struct InitialConnection;

#[derive(Component, Serialize, Deserialize)]
pub struct Connected;

/// A single merged log entry with its source entity and name.
pub struct MergedLogEntry {
    pub entity: Entity,
    pub name: String,
    pub line: LogLine,
}

const MERGED_LOG_CAP: usize = 500;

/// All log lines from all entities, merged and sorted by timestamp.
/// Append-only rolling buffer — new lines are appended incrementally,
/// old lines trimmed when the buffer exceeds `MERGED_LOG_CAP`.
#[derive(Resource, Default)]
pub struct MergedLogView {
    pub entries: Vec<MergedLogEntry>,
}

/// Drains tracing events into entity LogBuffers.
pub fn drain_tracing_logs(
    mut receiver: ResMut<ecsdk_tracing::TracingReceiver>,
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

/// Incrementally appends new log lines from all entities into the merged view.
pub fn build_merged_log_view(
    containers: Query<(Entity, &ContainerName, &LogBuffer), Without<SystemEntity>>,
    system_query: Query<(Entity, &ContainerName, &LogBuffer), With<SystemEntity>>,
    mut view: ResMut<MergedLogView>,
    mut cursor: Local<HashMap<Entity, usize>>,
) {
    let mut new_entries = Vec::new();

    for (entity, name, log_buf) in containers.iter().chain(system_query.iter()) {
        let seen = cursor.entry(entity).or_insert(0);
        for line in &log_buf.lines[*seen..] {
            new_entries.push(MergedLogEntry {
                entity,
                name: name.0.clone(),
                line: line.clone(),
            });
        }
        *seen = log_buf.lines.len();
    }

    if new_entries.is_empty() {
        return;
    }

    new_entries.sort_by_key(|e| e.line.timestamp);
    view.entries.extend(new_entries);

    if view.entries.len() > MERGED_LOG_CAP {
        let excess = view.entries.len() - MERGED_LOG_CAP;
        view.entries.drain(..excess);
    }
}
