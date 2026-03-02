use std::time::Instant;

use bevy_ecs::prelude::*;

#[derive(Component)]
pub struct ContainerName(pub String);

#[derive(Component)]
pub struct ImageRef(pub String);

/// Lower starts first; containers with the same order start in parallel.
#[derive(Component)]
pub struct StartOrder(pub u32);

#[derive(Component, PartialEq, Eq, Clone, Copy, Debug)]
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

#[derive(Component)]
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
#[derive(Component)]
pub struct SystemEntity;

/// A single merged log entry with its source entity and name.
pub struct MergedLogEntry {
    pub entity: Entity,
    pub name: String,
    pub line: LogLine,
}

/// All log lines from all entities, merged and sorted by timestamp.
#[derive(Resource, Default)]
pub struct MergedLogView {
    pub entries: Vec<MergedLogEntry>,
}

/// System that rebuilds the merged log view each frame.
pub fn build_merged_log_view(
    containers: Query<(Entity, &ContainerName, &LogBuffer), Without<SystemEntity>>,
    system_query: Query<(Entity, &ContainerName, &LogBuffer), With<SystemEntity>>,
    mut view: ResMut<MergedLogView>,
) {
    view.entries.clear();

    for (entity, name, log_buf) in &containers {
        for line in &log_buf.lines {
            view.entries.push(MergedLogEntry {
                entity,
                name: name.0.clone(),
                line: line.clone(),
            });
        }
    }
    for (entity, name, log_buf) in &system_query {
        for line in &log_buf.lines {
            view.entries.push(MergedLogEntry {
                entity,
                name: name.0.clone(),
                line: line.clone(),
            });
        }
    }

    view.entries.sort_by_key(|e| e.line.timestamp);
}

pub fn build_startup_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.add_systems(spawn_containers);
    schedule
}

fn spawn_containers(mut commands: Commands) {
    let containers = [
        ("postgres", "postgres:16", 0),
        ("redis", "redis:7", 0),
        ("api-server", "myapp/api:latest", 1),
        ("web-frontend", "myapp/web:latest", 2),
    ];

    for (name, image, order) in containers {
        commands.spawn((
            ContainerName(name.to_string()),
            ImageRef(image.to_string()),
            StartOrder(order),
            ContainerPhase::Pending,
            LogBuffer::default(),
        ));
    }

    // System entity for global messages
    commands.spawn((
        ContainerName("[system]".to_string()),
        LogBuffer::default(),
        SystemEntity,
    ));
}
