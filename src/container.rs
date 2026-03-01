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
        ));
    }
}
