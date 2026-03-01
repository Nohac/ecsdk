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
}

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
            handle.0.spawn(async move {
                let total = 100_000_000u64;
                for (i, delay) in delays.into_iter().enumerate() {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    let downloaded = total * (i as u64 + 1) / 10;
                    tx.send(move |world| {
                        if let Some(mut p) = world.get_mut::<DownloadProgress>(entity) {
                            p.downloaded = downloaded;
                            p.total = total;
                        }
                    });
                }
                tx.trigger(DownloadComplete(entity));
            });
        }
    }
}

/// Observer: DownloadComplete -> set phase to Starting, spawn boot task.
fn handle_download_complete(
    trigger: On<DownloadComplete>,
    mut commands: Commands,
    tx: Res<EventSender>,
    handle: Res<TokioHandle>,
) {
    let entity = trigger.event().0;
    commands.entity(entity).insert(ContainerPhase::Starting);

    let delay = rand::rng().random_range(500..=1500u64);
    let tx = tx.clone();
    handle.0.spawn(async move {
        tokio::time::sleep(Duration::from_millis(delay)).await;
        tx.trigger(BootComplete(entity));
    });
}

/// Observer: BootComplete -> set phase to Running. If all containers are
/// Running, set AppExit.
fn handle_boot_complete(
    trigger: On<BootComplete>,
    mut commands: Commands,
    all_phases: Query<&ContainerPhase>,
    mut exit: ResMut<AppExit>,
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
        println!("\nAll containers ready.");
        exit.0 = true;
    }
}

/// Observer: ShutdownAll -> set active containers to Stopping, spawn shutdown tasks.
fn handle_shutdown_all(
    _trigger: On<ShutdownAll>,
    mut commands: Commands,
    containers: Query<(Entity, &ContainerPhase)>,
    tx: Res<EventSender>,
    handle: Res<TokioHandle>,
) {
    println!("\nShutting down...");
    for (entity, phase) in &containers {
        match phase {
            ContainerPhase::Running
            | ContainerPhase::PullingImage
            | ContainerPhase::Starting
            | ContainerPhase::Pending => {
                commands.entity(entity).insert(ContainerPhase::Stopping);

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
    all_phases: Query<&ContainerPhase>,
    mut exit: ResMut<AppExit>,
) {
    let entity = trigger.event().0;
    commands.entity(entity).insert(ContainerPhase::Stopped);

    let not_stopped = all_phases
        .iter()
        .filter(|p| **p != ContainerPhase::Stopped)
        .count();

    if not_stopped <= 1 {
        println!("\nAll containers stopped.");
        exit.0 = true;
    }
}
