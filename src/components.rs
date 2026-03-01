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

#[derive(Component)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
}
