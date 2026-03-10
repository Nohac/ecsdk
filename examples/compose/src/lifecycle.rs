use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use ecsdk_core::{MessageQueue, ScheduleControl};
use ecsdk_tasks::SpawnTask;
use seldom_state::prelude::*;
use tracing::Instrument;

use crate::backend::ContainerBackend;
use crate::backend::ContainerRuntime;
use crate::container::*;
use crate::container::container_phase::{
    Failed, Pending, PullingImage, Running, Starting, Stopped, Stopping,
};
use crate::container::orchestrator_phase::{AllRunning, AllStopped, Deploying, ShuttingDown};
use crate::message::Message;

#[derive(Component)]
pub struct EntityError(pub anyhow::Error);

// ── Events and resources ──

#[derive(Event)]
pub struct ShutdownAll;

#[derive(Resource, Default)]
pub struct ShutdownRequested(pub bool);

/// Per-entity backend that knows which container it manages.
#[derive(Component, Clone)]
pub struct Backend(pub ContainerRuntime);

// ── Triggers ──

#[allow(clippy::type_complexity)]
fn predecessors_ready(
    In(entity): In<Entity>,
    this: Query<&StartOrder>,
    all: Query<(&StartOrder, Has<Running>, Has<Stopped>, Has<Failed>)>,
) -> bool {
    let Ok(order) = this.get(entity) else {
        return false;
    };
    all.iter()
        .all(|(other_order, is_running, is_stopped, is_failed)| {
            if other_order.0 < order.0 {
                is_running || is_stopped || is_failed
            } else {
                true
            }
        })
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

#[allow(clippy::type_complexity)]
fn all_containers_stopped(
    In(_entity): In<Entity>,
    containers: Query<(Has<Stopped>, Has<Failed>), (With<StartOrder>, Without<SystemEntity>)>,
) -> bool {
    !containers.is_empty() && containers.iter().all(|(s, f)| s || f)
}

// ── State machine builders ──

pub fn build_container_sm() -> StateMachine {
    StateMachine::default()
        .trans::<Pending, _>(predecessors_ready, PullingImage)
        .trans::<PullingImage, _>(done(Some(Done::Success)), Starting)
        .trans::<PullingImage, _>(done(Some(Done::Failure)), Failed)
        .trans::<Starting, _>(done(Some(Done::Success)), Running)
        .trans::<Starting, _>(done(Some(Done::Failure)), Failed)
        .trans::<OneOfState<(Pending, PullingImage, Starting, Running)>, _>(
            shutdown_requested,
            Stopping,
        )
        .trans::<Stopping, _>(done(Some(Done::Success)), Stopped)
        .trans::<Stopping, _>(done(Some(Done::Failure)), Failed)
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
    names: Query<&ContainerName>,
) {
    let entity = trigger.event_target();
    let Ok(backend) = backends.get(entity) else {
        return;
    };
    let backend = backend.0.clone();
    let container_name = names.get(entity).map(|n| n.0.clone()).unwrap_or_default();

    commands.entity(entity).insert(DownloadProgress {
        downloaded: 0,
        total: 0,
    });

    commands.entity(entity).spawn_task(move |cmd| {
        let entity = cmd.entity();
        let span = tracing::info_span!("container", entity_id = entity.to_bits());
        async move {
            let cmd_progress = cmd.clone();
            let result = backend
                .pull_image(
                    move |p| {
                        cmd_progress.send(move |world: &mut World| {
                            if let Some(mut dp) = world.get_mut::<DownloadProgress>(entity) {
                                dp.downloaded = p.downloaded;
                                dp.total = p.total;
                            }
                            world.tick();
                        });
                    },
                    |text| {
                        tracing::info!("{text}");
                    },
                )
                .await;
            match result {
                Ok(()) => cmd.send_state(Message::MarkDone { container_name }),
                Err(e) => {
                    cmd.send(move |world: &mut World| {
                        world
                            .entity_mut(entity)
                            .insert((EntityError(e), Done::Failure));
                    })
                    .wake();
                }
            }
        }
        .instrument(span)
    });
}

fn on_starting(
    trigger: On<Insert, Starting>,
    mut commands: Commands,
    backends: Query<&Backend>,
    names: Query<&ContainerName>,
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
    let container_name = names.get(entity).map(|n| n.0.clone()).unwrap_or_default();

    commands.entity(entity).spawn_task(move |cmd| {
        let entity = cmd.entity();
        let span = tracing::info_span!("container", entity_id = entity.to_bits());
        async move {
            let result = backend
                .boot_container(|text| {
                    tracing::info!("{text}");
                })
                .await;
            match result {
                Ok(()) => cmd.send_state(Message::MarkDone { container_name }),
                Err(e) => {
                    cmd.send(move |world: &mut World| {
                        world
                            .entity_mut(entity)
                            .insert((EntityError(e), Done::Failure));
                    })
                    .wake();
                }
            }
        }
        .instrument(span)
    });
}

fn on_running(trigger: On<Insert, Running>, mut logs: Query<&mut LogBuffer>) {
    let entity = trigger.event_target();
    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Container started");
    }
}

fn on_stopping(
    trigger: On<Insert, Stopping>,
    mut commands: Commands,
    backends: Query<&Backend>,
    names: Query<&ContainerName>,
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
    let container_name = names.get(entity).map(|n| n.0.clone()).unwrap_or_default();

    commands.entity(entity).spawn_task(move |cmd| async move {
        let entity = cmd.entity();
        match backend.stop_container().await {
            Ok(()) => cmd.send_state(Message::MarkDone { container_name }),
            Err(e) => {
                cmd.send(move |world: &mut World| {
                    world
                        .entity_mut(entity)
                        .insert((EntityError(e), Done::Failure));
                })
                .wake();
            }
        }
    });
}

fn on_stopped(trigger: On<Insert, Stopped>, mut logs: Query<&mut LogBuffer>) {
    let entity = trigger.event_target();
    if let Ok(mut log_buf) = logs.get_mut(entity) {
        log_buf.push("Container stopped");
    }
}

fn on_failed(
    trigger: On<Insert, Failed>,
    mut logs: Query<&mut LogBuffer>,
    errors: Query<&EntityError>,
) {
    let entity = trigger.event_target();
    if let Ok(err) = errors.get(entity)
        && let Ok(mut log_buf) = logs.get_mut(entity)
    {
        log_buf.push(format!("Error: {:#}", err.0));
    }
}

// ── Orchestrator observers ──

fn on_all_running(
    _trigger: On<Insert, AllRunning>,
    mut logs: Query<&mut LogBuffer>,
    system_entity: Query<Entity, With<SystemEntity>>,
) {
    if let Ok(sys) = system_entity.single()
        && let Ok(mut log_buf) = logs.get_mut(sys)
    {
        log_buf.push("All containers ready.");
    }
}

fn on_all_stopped(
    _trigger: On<Insert, AllStopped>,
    mut logs: Query<&mut LogBuffer>,
    system_entity: Query<Entity, With<SystemEntity>>,
    mut exit: ResMut<ecsdk_core::AppExit>,
) {
    if let Ok(sys) = system_entity.single()
        && let Ok(mut log_buf) = logs.get_mut(sys)
    {
        log_buf.push("All containers stopped.");
    }
    exit.0 = true;
}

// ── ShutdownAll handler ──

fn handle_shutdown_all(_trigger: On<ShutdownAll>, state_queue: Res<MessageQueue<Message>>) {
    state_queue.send(Message::RequestShutdown);
}

// ── Test-only plugin (state machines without side-effect observers) ──

/// Registers state machine infrastructure and resources without observers.
/// Use for pure state machine tests that don't need async tasks or CmdQueue.
pub struct LifecycleTestPlugin;

impl Plugin for LifecycleTestPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(StateMachinePlugin::default().schedule(PreUpdate));
        app.init_resource::<MergedLogView>();
        app.init_resource::<ShutdownRequested>();
    }
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
        app.add_observer(on_failed);

        // Orchestrator observers
        app.add_observer(on_all_running);
        app.add_observer(on_all_stopped);

        // ShutdownAll handler
        app.add_observer(handle_shutdown_all);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::container_phase::{
        Failed, Pending, PullingImage, Running, Starting, Stopped, Stopping,
    };
    use crate::container::orchestrator_phase::{AllRunning, AllStopped, Deploying, ShuttingDown};
    use crate::container::{ContainerName, ContainerPhase, OrchestratorPhase, StartOrder};

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LifecycleTestPlugin);
        app
    }

    fn spawn_container(app: &mut App, order: u32, state: impl Component + Clone) -> Entity {
        app.world_mut()
            .spawn((
                ContainerName(format!("test-{order}")),
                StartOrder(order),
                state,
                build_container_sm(),
                LogBuffer::default(),
            ))
            .id()
    }

    #[test]
    fn pending_transitions_to_pulling_when_no_predecessors() {
        let mut app = test_app();
        let entity = spawn_container(&mut app, 0, Pending);

        app.update();

        assert!(app.world().get::<PullingImage>(entity).is_some());
        assert_eq!(
            app.world().get::<ContainerPhase>(entity),
            Some(&ContainerPhase::PullingImage),
        );
    }

    #[test]
    fn pending_waits_for_predecessors() {
        let mut app = test_app();
        let _first = spawn_container(&mut app, 0, Pending);
        let second = spawn_container(&mut app, 1, Pending);

        app.update();

        // First moved to PullingImage, second still Pending (waiting on first)
        assert!(app.world().get::<Pending>(second).is_some());
    }

    #[test]
    fn pulling_transitions_to_starting_on_done() {
        let mut app = test_app();
        let entity = spawn_container(&mut app, 0, Pending);

        app.update(); // Pending → PullingImage

        app.world_mut().entity_mut(entity).insert(Done::Success);
        app.update(); // PullingImage → Starting

        assert!(app.world().get::<Starting>(entity).is_some());
    }

    #[test]
    fn full_lifecycle_pending_to_stopped() {
        let mut app = test_app();
        let entity = spawn_container(&mut app, 0, Pending);

        app.update(); // Pending → PullingImage
        assert!(app.world().get::<PullingImage>(entity).is_some());

        app.world_mut().entity_mut(entity).insert(Done::Success);
        app.update(); // PullingImage → Starting
        assert!(app.world().get::<Starting>(entity).is_some());

        app.world_mut().entity_mut(entity).insert(Done::Success);
        app.update(); // Starting → Running
        assert!(app.world().get::<Running>(entity).is_some());

        app.world_mut().resource_mut::<ShutdownRequested>().0 = true;
        app.update(); // Running → Stopping
        assert!(app.world().get::<Stopping>(entity).is_some());

        app.world_mut().entity_mut(entity).insert(Done::Success);
        app.update(); // Stopping → Stopped
        assert!(app.world().get::<Stopped>(entity).is_some());
        assert_eq!(
            app.world().get::<ContainerPhase>(entity),
            Some(&ContainerPhase::Stopped),
        );
    }

    #[test]
    fn shutdown_interrupts_pulling() {
        let mut app = test_app();
        let entity = spawn_container(&mut app, 0, Pending);

        app.update(); // Pending → PullingImage

        app.world_mut().resource_mut::<ShutdownRequested>().0 = true;
        app.update(); // PullingImage → Stopping

        assert!(app.world().get::<Stopping>(entity).is_some());
    }

    #[test]
    fn orchestrator_transitions_to_all_running() {
        let mut app = test_app();
        let c1 = spawn_container(&mut app, 0, Pending);
        let c2 = spawn_container(&mut app, 0, Pending);
        let _orch = app
            .world_mut()
            .spawn((
                OrchestratorPhase::Deploying,
                Deploying,
                build_orchestrator_sm(),
            ))
            .id();

        app.update(); // both Pending → PullingImage

        // Drive both to Running
        app.world_mut().entity_mut(c1).insert(Done::Success);
        app.world_mut().entity_mut(c2).insert(Done::Success);
        app.update(); // PullingImage → Starting

        app.world_mut().entity_mut(c1).insert(Done::Success);
        app.world_mut().entity_mut(c2).insert(Done::Success);
        app.update(); // Starting → Running

        app.update(); // orchestrator sees all Running → AllRunning

        assert!(app.world().get::<AllRunning>(_orch).is_some());
        assert_eq!(
            app.world().get::<OrchestratorPhase>(_orch),
            Some(&OrchestratorPhase::AllRunning)
        );
    }

    #[test]
    fn shutdown_completes_when_one_container_failed() {
        let mut app = test_app();
        let c1 = spawn_container(&mut app, 0, Pending);
        let c2 = spawn_container(&mut app, 0, Pending);
        let orch = app
            .world_mut()
            .spawn((
                OrchestratorPhase::Deploying,
                Deploying,
                build_orchestrator_sm(),
            ))
            .id();

        app.update(); // both Pending → PullingImage

        // c1 succeeds through to Running, c2 fails during pull
        app.world_mut().entity_mut(c1).insert(Done::Success);
        app.world_mut().entity_mut(c2).insert(Done::Failure);
        app.update(); // c1: PullingImage → Starting, c2: PullingImage → Failed

        assert!(app.world().get::<Starting>(c1).is_some());
        assert!(app.world().get::<Failed>(c2).is_some());
        assert_eq!(
            app.world().get::<ContainerPhase>(c2),
            Some(&ContainerPhase::Failed),
        );

        app.world_mut().entity_mut(c1).insert(Done::Success);
        app.update(); // c1: Starting → Running

        // Request shutdown
        app.world_mut().resource_mut::<ShutdownRequested>().0 = true;
        app.update(); // c1: Running → Stopping, c2 stays Failed, orch → ShuttingDown

        assert!(app.world().get::<Stopping>(c1).is_some());
        assert!(app.world().get::<Failed>(c2).is_some());
        assert!(app.world().get::<ShuttingDown>(orch).is_some());
        assert_eq!(
            app.world().get::<OrchestratorPhase>(orch),
            Some(&OrchestratorPhase::ShuttingDown)
        );

        app.world_mut().entity_mut(c1).insert(Done::Success);
        app.update(); // c1: Stopping → Stopped

        app.update(); // orchestrator sees all stopped/failed → AllStopped

        assert!(app.world().get::<Stopped>(c1).is_some());
        assert!(app.world().get::<Failed>(c2).is_some());
        assert!(app.world().get::<AllStopped>(orch).is_some());
        assert_eq!(
            app.world().get::<OrchestratorPhase>(orch),
            Some(&OrchestratorPhase::AllStopped)
        );
    }
}
