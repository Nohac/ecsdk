pub mod lifecycle;

use bevy_ecs::prelude::*;

use crate::components::*;

/// Builds the startup schedule: spawns the hardcoded container entities.
pub fn build_startup_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.add_systems(spawn_containers);
    schedule
}

/// Builds the update schedule: runs enforce_ordering.
pub fn build_update_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.add_systems(lifecycle::enforce_ordering);
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
