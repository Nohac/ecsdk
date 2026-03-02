use bevy_ecs::prelude::*;

use crate::backend::{ContainerBackend, ContainerRuntime};
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

/// Holds the container backend implementation as a shared resource.
#[derive(Resource, Clone)]
pub struct Backend(pub ContainerRuntime);

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
/// an async download task via the backend.
pub fn enforce_ordering(
    mut commands: Commands,
    pending: Query<(Entity, &ImageRef, &StartOrder, &ContainerPhase)>,
    all: Query<(&StartOrder, &ContainerPhase)>,
    backend: Res<Backend>,
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

            let backend = backend.0.clone();
            let tx = tx.clone();
            let image_name = image.0.clone();
            handle.0.spawn(async move {
                let (progress_tx, mut progress_rx) =
                    tokio::sync::mpsc::unbounded_channel::<crate::backend::PullProgress>();
                let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel();

                // Forward progress updates to ECS
                let tx2 = tx.clone();
                let progress_fwd = tokio::spawn(async move {
                    while let Some(p) = progress_rx.recv().await {
                        let tx3 = tx2.clone();
                        tx3.send(move |world| {
                            if let Some(mut dp) = world.get_mut::<DownloadProgress>(entity) {
                                dp.downloaded = p.downloaded;
                                dp.total = p.total;
                            }
                        });
                    }
                });

                // Forward log lines to ECS
                let tx2 = tx.clone();
                let log_fwd = tokio::spawn(async move {
                    while let Some(text) = log_rx.recv().await {
                        tx2.trigger(AsyncLogLine { entity, text });
                    }
                });

                let _ = backend.pull_image(&image_name, progress_tx, log_tx).await;
                let _ = progress_fwd.await;
                let _ = log_fwd.await;
                tx.trigger(DownloadComplete(entity));
            });
        }
    }
}

/// Observer: DownloadComplete -> set phase to Starting, spawn boot task via backend.
fn handle_download_complete(
    trigger: On<DownloadComplete>,
    mut commands: Commands,
    names: Query<&ContainerName>,
    mut logs: Query<&mut LogBuffer>,
    backend: Res<Backend>,
    tx: Res<EventSender>,
    handle: Res<TokioHandle>,
) {
    let entity = trigger.event().0;
    commands.entity(entity).insert(ContainerPhase::Starting);

    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Starting container...");
    }

    let container_name = names.get(entity).map(|n| n.0.clone()).unwrap_or_default();

    let backend = backend.0.clone();
    let tx = tx.clone();
    handle.0.spawn(async move {
        let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel();

        // Forward log lines to ECS
        let tx2 = tx.clone();
        let log_fwd = tokio::spawn(async move {
            while let Some(text) = log_rx.recv().await {
                tx2.trigger(AsyncLogLine { entity, text });
            }
        });

        let _ = backend.boot_container(&container_name, log_tx).await;
        let _ = log_fwd.await;
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

/// Observer: ShutdownAll -> set active containers to Stopping, spawn shutdown tasks via backend.
#[allow(clippy::too_many_arguments)]
fn handle_shutdown_all(
    _trigger: On<ShutdownAll>,
    mut commands: Commands,
    containers: Query<(Entity, &ContainerName, &ContainerPhase), Without<SystemEntity>>,
    mut logs: Query<&mut LogBuffer>,
    system_entity: Query<Entity, With<SystemEntity>>,
    backend: Res<Backend>,
    tx: Res<EventSender>,
    handle: Res<TokioHandle>,
) {
    if let Ok(sys) = system_entity.single()
        && let Ok(mut log_buf) = logs.get_mut(sys)
    {
        log_buf.push("Shutting down...");
    }

    for (entity, name, phase) in &containers {
        match phase {
            ContainerPhase::Running
            | ContainerPhase::PullingImage
            | ContainerPhase::Starting
            | ContainerPhase::Pending => {
                commands.entity(entity).insert(ContainerPhase::Stopping);

                if let Ok(mut log_buf) = logs.get_mut(entity) {
                    log_buf.push("Stopping container...");
                }

                let backend = backend.0.clone();
                let container_name = name.0.clone();
                let tx = tx.clone();
                handle.0.spawn(async move {
                    let _ = backend.stop_container(&container_name).await;
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
