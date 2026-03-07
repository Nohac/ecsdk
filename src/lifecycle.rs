use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use seldom_state::prelude::*;

use crate::backend::ContainerBackend;
use crate::backend::ContainerRuntime;
use crate::bridge::AppExit;
use crate::container::*;
use crate::task::{CommandSender, SpawnTask};

// ── State components (container lifecycle) ──

#[derive(Component, Clone)]
pub struct Pending;

#[derive(Component, Clone)]
pub struct PullingImage;

#[derive(Component, Clone)]
pub struct Starting;

#[derive(Component, Clone)]
pub struct Running;

#[derive(Component, Clone)]
pub struct Stopping;

#[derive(Component, Clone)]
pub struct Stopped;

// ── State components (orchestrator) ──

#[derive(Component, Clone)]
pub struct Deploying;

#[derive(Component, Clone)]
pub struct AllRunning;

#[derive(Component, Clone)]
pub struct ShuttingDown;

#[derive(Component, Clone)]
pub struct AllStopped;

// ── Events and resources ──

#[derive(Event)]
pub struct ShutdownAll;

#[derive(Resource, Default)]
pub struct ShutdownRequested(pub bool);

/// Per-entity backend that knows which container it manages.
#[derive(Component, Clone)]
pub struct Backend(pub ContainerRuntime);

// ── Triggers ──

fn predecessors_ready(
    In(entity): In<Entity>,
    this: Query<&StartOrder>,
    all: Query<(&StartOrder, Has<Running>, Has<Stopped>)>,
) -> bool {
    let Ok(order) = this.get(entity) else {
        return false;
    };
    all.iter().all(|(other_order, is_running, is_stopped)| {
        if other_order.0 < order.0 {
            is_running || is_stopped
        } else {
            true
        }
    })
}

fn has_done(In(entity): In<Entity>, dones: Query<&Done>) -> bool {
    dones.get(entity).is_ok()
}

fn shutdown_requested(In(_entity): In<Entity>, flag: Res<ShutdownRequested>) -> bool {
    flag.0
}

fn all_containers_running(
    In(_entity): In<Entity>,
    containers: Query<Has<Running>, (With<StartOrder>, Without<SystemEntity>)>,
) -> bool {
    !containers.is_empty() && containers.iter().all(|r| r)
}

fn all_containers_stopped(
    In(_entity): In<Entity>,
    containers: Query<Has<Stopped>, (With<StartOrder>, Without<SystemEntity>)>,
) -> bool {
    !containers.is_empty() && containers.iter().all(|s| s)
}

// ── State machine builders ──

pub fn build_container_sm() -> StateMachine {
    StateMachine::default()
        .trans::<Pending, _>(predecessors_ready, PullingImage)
        .on_enter::<PullingImage>(|e| {
            e.insert(ContainerPhase::PullingImage);
        })
        .trans::<PullingImage, _>(has_done, Starting)
        .on_enter::<Starting>(|e| {
            e.insert(ContainerPhase::Starting);
        })
        .trans::<Starting, _>(has_done, Running)
        .on_enter::<Running>(|e| {
            e.insert(ContainerPhase::Running);
        })
        .trans::<OneOfState<(Pending, PullingImage, Starting, Running)>, _>(
            shutdown_requested,
            Stopping,
        )
        .on_enter::<Stopping>(|e| {
            e.insert(ContainerPhase::Stopping);
        })
        .trans::<Stopping, _>(has_done, Stopped)
        .on_enter::<Stopped>(|e| {
            e.insert(ContainerPhase::Stopped);
        })
        .set_trans_logging(true)
}

pub fn build_orchestrator_sm() -> StateMachine {
    StateMachine::default()
        .trans::<Deploying, _>(all_containers_running, AllRunning)
        .trans::<OneOfState<(Deploying, AllRunning)>, _>(shutdown_requested, ShuttingDown)
        .trans::<ShuttingDown, _>(all_containers_stopped, AllStopped)
        .set_trans_logging(true)
}

// ── OnInsert observers (side effects on state entry) ──

fn on_pulling_image(
    trigger: On<Insert, PullingImage>,
    mut commands: Commands,
    backends: Query<&Backend>,
) {
    let entity = trigger.event_target();
    let Ok(backend) = backends.get(entity) else {
        return;
    };
    let backend = backend.0.clone();

    commands.entity(entity).insert(DownloadProgress {
        downloaded: 0,
        total: 0,
    });

    commands.entity(entity).spawn_task(move |cmd| async move {
        let entity = cmd.entity();
        let cmd_progress = cmd.clone();
        let cmd_logs = cmd.clone();
        let _ = backend
            .pull_image(
                move |p| {
                    cmd_progress.push(move |world: &mut World| {
                        if let Some(mut dp) = world.get_mut::<DownloadProgress>(entity) {
                            dp.downloaded = p.downloaded;
                            dp.total = p.total;
                        }
                    });
                },
                move |text| {
                    cmd_logs.push(move |world: &mut World| {
                        if let Some(mut log_buf) = world.get_mut::<LogBuffer>(entity) {
                            log_buf.push(text);
                        }
                    });
                },
            )
            .await;
        cmd.push(move |world: &mut World| {
            if world.get_entity(entity).is_ok() {
                world.entity_mut(entity).insert(Done::Success);
            }
        });
    });
}

fn on_starting(
    trigger: On<Insert, Starting>,
    mut commands: Commands,
    backends: Query<&Backend>,
    mut logs: Query<&mut LogBuffer>,
) {
    let entity = trigger.event_target();

    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Starting container...");
    }

    let Ok(backend) = backends.get(entity) else {
        return;
    };
    let backend = backend.0.clone();

    commands.entity(entity).spawn_task(move |cmd| async move {
        let entity = cmd.entity();
        let cmd_logs = cmd.clone();
        let _ = backend
            .boot_container(move |text| {
                cmd_logs.push(move |world: &mut World| {
                    if let Some(mut log_buf) = world.get_mut::<LogBuffer>(entity) {
                        log_buf.push(text);
                    }
                });
            })
            .await;
        cmd.push(move |world: &mut World| {
            if world.get_entity(entity).is_ok() {
                world.entity_mut(entity).insert(Done::Success);
            }
        });
    });
}

fn on_running(
    _trigger: On<Insert, Running>,
    mut logs: Query<&mut LogBuffer>,
    wake: Res<CommandSender>,
) {
    let entity = _trigger.event_target();
    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Container started");
    }
    wake.send(|_: &mut World| {});
}

fn on_stopping(
    trigger: On<Insert, Stopping>,
    mut commands: Commands,
    backends: Query<&Backend>,
    mut logs: Query<&mut LogBuffer>,
) {
    let entity = trigger.event_target();

    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Stopping container...");
    }

    let Ok(backend) = backends.get(entity) else {
        return;
    };
    let backend = backend.0.clone();

    commands.entity(entity).spawn_task(move |cmd| async move {
        let entity = cmd.entity();
        let _ = backend.stop_container().await;
        cmd.push(move |world: &mut World| {
            if world.get_entity(entity).is_ok() {
                world.entity_mut(entity).insert(Done::Success);
            }
        });
    });
}

fn on_stopped(
    _trigger: On<Insert, Stopped>,
    mut logs: Query<&mut LogBuffer>,
    wake: Res<CommandSender>,
) {
    let entity = _trigger.event_target();
    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Container stopped");
    }
    wake.send(|_: &mut World| {});
}

// ── Orchestrator observers ──

fn on_all_running(
    _trigger: On<Insert, AllRunning>,
    mut logs: Query<&mut LogBuffer>,
    system_entity: Query<Entity, With<SystemEntity>>,
    wake: Res<CommandSender>,
) {
    if let Ok(sys) = system_entity.single()
        && let Ok(mut log_buf) = logs.get_mut(sys)
    {
        log_buf.push("All containers ready.");
    }
    wake.send(|_: &mut World| {});
}

fn on_all_stopped(
    _trigger: On<Insert, AllStopped>,
    mut logs: Query<&mut LogBuffer>,
    system_entity: Query<Entity, With<SystemEntity>>,
    mut exit: ResMut<AppExit>,
    wake: Res<CommandSender>,
) {
    if let Ok(sys) = system_entity.single()
        && let Ok(mut log_buf) = logs.get_mut(sys)
    {
        log_buf.push("All containers stopped.");
    }
    exit.0 = true;
    wake.send(|_: &mut World| {});
}

// ── ShutdownAll handler ──

fn handle_shutdown_all(
    _trigger: On<ShutdownAll>,
    cmd: Res<CommandSender>,
) {
    cmd.send(|world: &mut World| {
        world.resource_mut::<ShutdownRequested>().0 = true;
        if let Some(sys) = world
            .query_filtered::<Entity, With<SystemEntity>>()
            .iter(world)
            .next()
            && let Some(mut log_buf) = world.get_mut::<LogBuffer>(sys)
        {
            log_buf.push("Shutting down...");
        }
    });
}

// ── Plugin ──

pub struct LifecyclePlugin;

impl Plugin for LifecyclePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(StateMachinePlugin::default().schedule(PreUpdate));
        app.init_resource::<MergedLogView>();
        app.init_resource::<ShutdownRequested>();

        // Container lifecycle observers
        app.add_observer(on_pulling_image);
        app.add_observer(on_starting);
        app.add_observer(on_running);
        app.add_observer(on_stopping);
        app.add_observer(on_stopped);

        // Orchestrator observers
        app.add_observer(on_all_running);
        app.add_observer(on_all_stopped);

        // ShutdownAll handler
        app.add_observer(handle_shutdown_all);
    }
}
