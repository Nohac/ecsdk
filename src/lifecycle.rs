use std::time::Duration;

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::bridge::{AppExit, EventSender, TokioHandle};
use crate::container::*;

// ECS trigger events — co-located with the observers that handle them.

#[derive(Event)]
pub struct DownloadComplete(pub Entity);

#[derive(Event)]
pub struct BootComplete(pub Entity);

#[derive(Event)]
pub struct ShutdownAll;

#[derive(Event)]
pub struct ShutdownComplete(pub Entity);

/// Event carrying a log line from an async task back to an entity's LogBuffer.
#[derive(Event)]
pub struct AsyncLogLine {
    pub entity: Entity,
    pub text: String,
}

pub fn build_update_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.add_systems(enforce_ordering);
    schedule
}

pub fn register_observers(world: &mut World) {
    world.add_observer(handle_download_complete);
    world.add_observer(handle_boot_complete);
    world.add_observer(handle_shutdown_all);
    world.add_observer(handle_shutdown_complete);
    world.add_observer(handle_async_log_line);
}

/// Queries Pending containers. If all containers with a lower StartOrder
/// are >= Running, transitions this container to PullingImage and spawns
/// an async download task.
pub fn enforce_ordering(
    mut commands: Commands,
    pending: Query<(Entity, &ImageRef, &StartOrder, &ContainerPhase)>,
    all: Query<(&StartOrder, &ContainerPhase)>,
    tx: Res<EventSender>,
    handle: Res<TokioHandle>,
) {
    for (entity, image, order, phase) in &pending {
        if *phase != ContainerPhase::Pending {
            continue;
        }

        let predecessors_ready = all.iter().all(|(other_order, other_phase)| {
            if other_order.0 < order.0 {
                matches!(
                    other_phase,
                    ContainerPhase::Running | ContainerPhase::Stopped
                )
            } else {
                true
            }
        });

        if predecessors_ready {
            commands.entity(entity).insert((
                ContainerPhase::PullingImage,
                DownloadProgress {
                    downloaded: 0,
                    total: 0,
                },
            ));

            // Pre-generate random delays (ThreadRng is !Send)
            let mut rng = rand::rng();
            let delays: Vec<u64> = (0..10).map(|_| rng.random_range(200..=500)).collect();

            let tx = tx.clone();
            let image_name = image.0.clone();
            handle.0.spawn(async move {
                // Log the start of the pull
                tx.trigger(AsyncLogLine {
                    entity,
                    text: format!("Pulling {image_name}..."),
                });

                let total = 100_000_000u64;
                for (i, delay) in delays.into_iter().enumerate() {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    let downloaded = total * (i as u64 + 1) / 10;
                    let tx2 = tx.clone();
                    tx2.send(move |world| {
                        if let Some(mut p) = world.get_mut::<DownloadProgress>(entity) {
                            p.downloaded = downloaded;
                            p.total = total;
                        }
                    });
                }
                tx.trigger(AsyncLogLine {
                    entity,
                    text: "Pull complete".to_string(),
                });
                tx.trigger(DownloadComplete(entity));
            });
        }
    }
}

/// Observer: DownloadComplete -> set phase to Starting, spawn boot task.
fn handle_download_complete(
    trigger: On<DownloadComplete>,
    mut commands: Commands,
    names: Query<&ContainerName>,
    mut logs: Query<&mut LogBuffer>,
    tx: Res<EventSender>,
    handle: Res<TokioHandle>,
) {
    let entity = trigger.event().0;
    commands.entity(entity).insert(ContainerPhase::Starting);

    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Starting container...");
    }

    let container_name = names.get(entity).map(|n| n.0.clone()).unwrap_or_default();

    let delay = rand::rng().random_range(500..=1500u64);
    let tx = tx.clone();
    handle.0.spawn(async move {
        // Simulated container stdout based on service type
        let boot_lines = match container_name.as_str() {
            "postgres" => vec![
                ("PostgreSQL init process complete", 200),
                ("LOG: listening on 0.0.0.0:5432", 300),
            ],
            "redis" => vec![
                ("oO0OoO0Oo Redis is starting oO0OoO0Oo", 150),
                ("Ready to accept connections on port 6379", 250),
            ],
            "api-server" => vec![
                ("Connecting to database...", 300),
                ("Server listening on :8080", 400),
            ],
            "web-frontend" => vec![
                ("Compiling assets...", 400),
                ("Serving on http://0.0.0.0:3000", 300),
            ],
            _ => vec![],
        };

        for (text, extra_delay) in boot_lines {
            tokio::time::sleep(Duration::from_millis(extra_delay)).await;
            tx.trigger(AsyncLogLine {
                entity,
                text: text.to_string(),
            });
        }

        tokio::time::sleep(Duration::from_millis(delay)).await;
        tx.trigger(BootComplete(entity));
    });
}

/// Observer: BootComplete -> set phase to Running. If all containers are
/// Running, log and set AppExit.
fn handle_boot_complete(
    trigger: On<BootComplete>,
    mut commands: Commands,
    all_phases: Query<&ContainerPhase, Without<SystemEntity>>,
    mut logs: Query<&mut LogBuffer>,
    system_entity: Query<Entity, With<SystemEntity>>,
    mut exit: ResMut<AppExit>,
) {
    let entity = trigger.event().0;
    commands.entity(entity).insert(ContainerPhase::Running);

    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Container started");
    }

    // The triggered entity still has its old phase (Starting) since commands
    // are deferred. Count how many are not yet Running — if only 1 (this one),
    // then after this command applies, all will be Running.
    let not_running = all_phases
        .iter()
        .filter(|p| **p != ContainerPhase::Running)
        .count();

    if not_running <= 1 {
        if let Ok(sys) = system_entity.single()
            && let Ok(mut log_buf) = logs.get_mut(sys)
        {
            log_buf.push("All containers ready.");
        }
        exit.0 = true;
    }
}

/// Observer: ShutdownAll -> set active containers to Stopping, spawn shutdown tasks.
fn handle_shutdown_all(
    _trigger: On<ShutdownAll>,
    mut commands: Commands,
    containers: Query<(Entity, &ContainerPhase), Without<SystemEntity>>,
    mut logs: Query<&mut LogBuffer>,
    system_entity: Query<Entity, With<SystemEntity>>,
    tx: Res<EventSender>,
    handle: Res<TokioHandle>,
) {
    if let Ok(sys) = system_entity.single()
        && let Ok(mut log_buf) = logs.get_mut(sys)
    {
        log_buf.push("Shutting down...");
    }

    for (entity, phase) in &containers {
        match phase {
            ContainerPhase::Running
            | ContainerPhase::PullingImage
            | ContainerPhase::Starting
            | ContainerPhase::Pending => {
                commands.entity(entity).insert(ContainerPhase::Stopping);

                if let Ok(mut log_buf) = logs.get_mut(entity) {
                    log_buf.push("Stopping container...");
                }

                let delay = rand::rng().random_range(200..=800u64);
                let tx = tx.clone();
                handle.0.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    tx.trigger(ShutdownComplete(entity));
                });
            }
            _ => {}
        }
    }
}

/// Observer: ShutdownComplete -> set phase to Stopped. If all stopped, set AppExit.
fn handle_shutdown_complete(
    trigger: On<ShutdownComplete>,
    mut commands: Commands,
    all_phases: Query<&ContainerPhase, Without<SystemEntity>>,
    mut logs: Query<&mut LogBuffer>,
    system_entity: Query<Entity, With<SystemEntity>>,
    mut exit: ResMut<AppExit>,
) {
    let entity = trigger.event().0;
    commands.entity(entity).insert(ContainerPhase::Stopped);

    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Container stopped");
    }

    let not_stopped = all_phases
        .iter()
        .filter(|p| **p != ContainerPhase::Stopped)
        .count();

    if not_stopped <= 1 {
        if let Ok(sys) = system_entity.single()
            && let Ok(mut log_buf) = logs.get_mut(sys)
        {
            log_buf.push("All containers stopped.");
        }
        exit.0 = true;
    }
}

/// Observer: AsyncLogLine -> push a log line into the target entity's LogBuffer.
fn handle_async_log_line(trigger: On<AsyncLogLine>, mut logs: Query<&mut LogBuffer>) {
    let event = trigger.event();
    if let Ok(mut log_buf) = logs.get_mut(event.entity) {
        log_buf.push(&event.text);
    }
}
