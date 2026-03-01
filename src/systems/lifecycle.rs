use std::time::Duration;

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::components::*;
use crate::events::*;
use crate::resources::{EventSender, TokioHandle};

/// Queries Pending containers. If all containers with a lower StartOrder
/// are >= Running, transitions this container to PullingImage and spawns
/// an async download task.
pub fn enforce_ordering(
    mut commands: Commands,
    pending: Query<(Entity, &StartOrder, &ContainerPhase)>,
    all: Query<(&StartOrder, &ContainerPhase)>,
    tx: Res<EventSender>,
    handle: Res<TokioHandle>,
) {
    for (entity, order, phase) in &pending {
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
            commands
                .entity(entity)
                .insert((ContainerPhase::PullingImage, DownloadProgress {
                    downloaded: 0,
                    total: 0,
                }));

            // Pre-generate random delays (ThreadRng is !Send)
            let mut rng = rand::rng();
            let delays: Vec<u64> = (0..10).map(|_| rng.random_range(200..=500)).collect();

            let tx = tx.0.clone();
            handle.0.spawn(async move {
                let total = 100_000_000u64;
                for (i, delay) in delays.into_iter().enumerate() {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    let _ = tx.send(AppEvent::DownloadProgress {
                        entity,
                        downloaded: total * (i as u64 + 1) / 10,
                        total,
                    });
                }
                let _ = tx.send(AppEvent::DownloadComplete(entity));
            });
        }
    }
}

/// Observer: DownloadComplete -> set phase to Starting, spawn boot task.
pub fn handle_download_complete(
    trigger: On<DownloadCompleteEcs>,
    mut commands: Commands,
    tx: Res<EventSender>,
    handle: Res<TokioHandle>,
) {
    let entity = trigger.event().0;
    commands.entity(entity).insert(ContainerPhase::Starting);

    let delay = rand::rng().random_range(500..=1500u64);
    let tx = tx.0.clone();
    handle.0.spawn(async move {
        tokio::time::sleep(Duration::from_millis(delay)).await;
        let _ = tx.send(AppEvent::BootComplete(entity));
    });
}

/// Observer: BootComplete -> set phase to Running. If all containers are
/// Running, send AllContainersReady.
pub fn handle_boot_complete(
    trigger: On<BootCompleteEcs>,
    mut commands: Commands,
    all_phases: Query<&ContainerPhase>,
    tx: Res<EventSender>,
) {
    let entity = trigger.event().0;
    commands.entity(entity).insert(ContainerPhase::Running);

    // The triggered entity still has its old phase (Starting) since commands
    // are deferred. Count how many are not yet Running — if only 1 (this one),
    // then after this command applies, all will be Running.
    let not_running = all_phases
        .iter()
        .filter(|p| **p != ContainerPhase::Running)
        .count();

    if not_running <= 1 {
        let _ = tx.0.send(AppEvent::AllContainersReady);
    }
}

/// Observer: ShutdownAll -> set active containers to Stopping, spawn shutdown tasks.
pub fn handle_shutdown_all(
    _trigger: On<ShutdownAllEcs>,
    mut commands: Commands,
    containers: Query<(Entity, &ContainerPhase)>,
    tx: Res<EventSender>,
    handle: Res<TokioHandle>,
) {
    for (entity, phase) in &containers {
        match phase {
            ContainerPhase::Running
            | ContainerPhase::PullingImage
            | ContainerPhase::Starting
            | ContainerPhase::Pending => {
                commands.entity(entity).insert(ContainerPhase::Stopping);

                let delay = rand::rng().random_range(200..=800u64);
                let tx = tx.0.clone();
                handle.0.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    let _ = tx.send(AppEvent::ShutdownComplete(entity));
                });
            }
            _ => {}
        }
    }
}

/// Observer: ShutdownComplete -> set phase to Stopped.
pub fn handle_shutdown_complete(trigger: On<ShutdownCompleteEcs>, mut commands: Commands) {
    let entity = trigger.event().0;
    commands.entity(entity).insert(ContainerPhase::Stopped);
}
