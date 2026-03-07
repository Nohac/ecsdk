use bevy::ecs::prelude::*;

use crate::backend::ContainerBackend;
use crate::backend::ContainerRuntime;
use crate::bridge::AppExit;
use crate::container::*;
use crate::task::SpawnTask;
use bevy::app::prelude::*;

// ECS trigger events — co-located with the observers that handle them.

#[derive(Event)]
pub struct DownloadComplete(pub Entity);

#[derive(Event)]
pub struct BootComplete(pub Entity);

#[derive(Event)]
pub struct ShutdownAll;

#[derive(Event)]
pub struct ShutdownComplete(pub Entity);

/// Per-entity backend that knows which container it manages.
#[derive(Component, Clone)]
pub struct Backend(pub ContainerRuntime);

pub struct LifecyclePlugin;

impl Plugin for LifecyclePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MergedLogView>();
        app.world_mut().add_observer(handle_download_complete);
        app.world_mut().add_observer(handle_boot_complete);
        app.world_mut().add_observer(handle_shutdown_all);
        app.world_mut().add_observer(handle_shutdown_complete);
        app.add_systems(Update, enforce_ordering)
            .add_systems(Update, check_all_running)
            .add_systems(Update, check_all_stopped);
    }
}

/// Queries Pending containers. If all containers with a lower StartOrder
/// are >= Running, transitions this container to PullingImage and spawns
/// an async download task via the backend.
pub fn enforce_ordering(
    mut commands: Commands,
    pending: Query<(Entity, &StartOrder, &ContainerPhase, &Backend)>,
    all: Query<(&StartOrder, &ContainerPhase)>,
) {
    for (entity, order, phase, backend) in &pending {
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
            let backend = backend.0.clone();

            commands
                .entity(entity)
                .insert((
                    ContainerPhase::PullingImage,
                    DownloadProgress {
                        downloaded: 0,
                        total: 0,
                    },
                ))
                .spawn_task(move |cmd| async move {
                    let (progress_tx, mut progress_rx) =
                        tokio::sync::mpsc::unbounded_channel::<crate::backend::PullProgress>();
                    let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

                    let entity = cmd.entity();
                    let cmd_progress = cmd.clone();
                    let cmd_logs = cmd.clone();
                    let _ = tokio::join!(
                        backend.pull_image(progress_tx, log_tx),
                        async move {
                            while let Some(p) = progress_rx.recv().await {
                                cmd_progress.push(move |world: &mut World| {
                                    if let Some(mut dp) = world.get_mut::<DownloadProgress>(entity)
                                    {
                                        dp.downloaded = p.downloaded;
                                        dp.total = p.total;
                                    }
                                });
                            }
                        },
                        async move {
                            while let Some(text) = log_rx.recv().await {
                                cmd_logs.push(move |world: &mut World| {
                                    if let Some(mut log_buf) = world.get_mut::<LogBuffer>(entity) {
                                        log_buf.push(text);
                                    }
                                });
                            }
                        },
                    );
                    cmd.trigger(DownloadComplete(entity));
                });
        }
    }
}

/// Observer: DownloadComplete -> set phase to Starting, spawn boot task via backend.
fn handle_download_complete(
    trigger: On<DownloadComplete>,
    mut commands: Commands,
    mut logs: Query<&mut LogBuffer>,
    backends: Query<&Backend>,
) {
    let entity = trigger.event().0;

    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Starting container...");
    }

    let backend = backends.get(entity).unwrap().0.clone();

    commands
        .entity(entity)
        .insert(ContainerPhase::Starting)
        .spawn_task(move |cmd| async move {
            let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

            let entity = cmd.entity();
            let cmd_logs = cmd.clone();
            let _ = tokio::join!(backend.boot_container(log_tx), async move {
                while let Some(text) = log_rx.recv().await {
                    cmd_logs.push(move |world: &mut World| {
                        if let Some(mut log_buf) = world.get_mut::<LogBuffer>(entity) {
                            log_buf.push(text);
                        }
                    });
                }
            },);
            cmd.trigger(BootComplete(entity));
        });
}

/// Observer: BootComplete -> set phase to Running.
fn handle_boot_complete(
    trigger: On<BootComplete>,
    mut commands: Commands,
    mut logs: Query<&mut LogBuffer>,
) {
    let entity = trigger.event().0;
    commands.entity(entity).insert(ContainerPhase::Running);

    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Container started");
    }
}

/// Observer: ShutdownAll -> set active containers to Stopping, spawn shutdown tasks via backend.
fn handle_shutdown_all(
    _trigger: On<ShutdownAll>,
    mut commands: Commands,
    containers: Query<(Entity, &Backend, &ContainerPhase), Without<SystemEntity>>,
    mut logs: Query<&mut LogBuffer>,
    system_entity: Query<Entity, With<SystemEntity>>,
) {
    if let Ok(sys) = system_entity.single()
        && let Ok(mut log_buf) = logs.get_mut(sys)
    {
        log_buf.push("Shutting down...");
    }

    for (entity, backend, phase) in &containers {
        match phase {
            ContainerPhase::Running
            | ContainerPhase::PullingImage
            | ContainerPhase::Starting
            | ContainerPhase::Pending => {
                if let Ok(mut log_buf) = logs.get_mut(entity) {
                    log_buf.push("Stopping container...");
                }

                let backend = backend.0.clone();

                commands
                    .entity(entity)
                    .insert(ContainerPhase::Stopping)
                    .spawn_task(move |cmd| async move {
                        let _ = backend.stop_container().await;
                        cmd.trigger(ShutdownComplete(cmd.entity()));
                    });
            }
            _ => {}
        }
    }
}

/// Observer: ShutdownComplete -> set phase to Stopped.
fn handle_shutdown_complete(
    trigger: On<ShutdownComplete>,
    mut commands: Commands,
    mut logs: Query<&mut LogBuffer>,
) {
    let entity = trigger.event().0;
    commands.entity(entity).insert(ContainerPhase::Stopped);

    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Container stopped");
    }
}

/// System: if all containers are Running, log and exit.
pub fn check_all_running(
    all_phases: Query<&ContainerPhase, Without<SystemEntity>>,
    mut logs: Query<&mut LogBuffer>,
    system_entity: Query<Entity, With<SystemEntity>>,
) {
    if all_phases.iter().all(|p| *p == ContainerPhase::Running)
        && let Ok(sys) = system_entity.single()
        && let Ok(mut log_buf) = logs.get_mut(sys)
    {
        log_buf.push("All containers ready.");
    }
}

/// System: if all containers are Stopped, log and exit.
pub fn check_all_stopped(
    all_phases: Query<&ContainerPhase, Without<SystemEntity>>,
    mut logs: Query<&mut LogBuffer>,
    system_entity: Query<Entity, With<SystemEntity>>,
    mut exit: ResMut<AppExit>,
) {
    if exit.0 || all_phases.is_empty() {
        return;
    }
    if all_phases.iter().all(|p| *p == ContainerPhase::Stopped) {
        if let Ok(sys) = system_entity.single()
            && let Ok(mut log_buf) = logs.get_mut(sys)
        {
            log_buf.push("All containers stopped.");
        }
        exit.0 = true;
    }
}
