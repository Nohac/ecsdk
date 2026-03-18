use std::collections::HashMap;

use ecsdk::prelude::*;
use ecsdk::core::{AppExit, MessageQueue, WakeSignal};
use ecsdk::tasks::SpawnTask;
use tokio::signal::ctrl_c;
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::backend_mock::MockBackend;
use crate::container::*;
use crate::lifecycle::*;
use crate::message::Message;

fn send_log_events(
    mut commands: Commands,
    query: Query<(Entity, &LogBuffer)>,
    mut tracked: Local<HashMap<Entity, usize>>,
) {
    for (entity, log_buf) in &query {
        let sent = tracked.get(&entity).copied().unwrap_or(0);
        for line in &log_buf.lines[sent..] {
            commands.server_trigger(ToClients {
                mode: SendMode::Broadcast,
                message: crate::protocol::LogEvent {
                    container_entity: entity,
                    text: line.text.clone(),
                },
            });
        }
        tracked.insert(entity, log_buf.lines.len());
    }
}

fn send_exit_notice(mut commands: Commands, exit: Res<AppExit>, mut sent: Local<bool>) {
    if exit.0 && !*sent {
        commands.server_trigger(ToClients {
            mode: SendMode::Broadcast,
            message: crate::protocol::ServerExitNotice,
        });
        *sent = true;
    }
}

fn handle_shutdown_request(
    _trigger: On<FromClient<crate::protocol::ShutdownRequest>>,
    mut commands: Commands,
) {
    commands.trigger(ShutdownAll);
}

// ---------------------------------------------------------------------------
// DaemonPlugin — bundles all server-side registration
// ---------------------------------------------------------------------------

pub struct DaemonPlugin;

impl Plugin for DaemonPlugin {
    fn build(&self, app: &mut App) {
        // Lifecycle + log/exit broadcast
        app.add_plugins(LifecyclePlugin);
        app.add_systems(
            PreUpdate,
            crate::container::drain_tracing_logs
                .run_if(resource_exists::<ecsdk::tracing::TracingReceiver>),
        );
        app.add_systems(Update, (send_log_events, send_exit_notice));
        app.add_observer(handle_shutdown_request);

        // Ctrl+C triggers graceful shutdown
        app.add_systems(Startup, spawn_ctrl_c_handler);
    }
}

fn spawn_ctrl_c_handler(mut commands: Commands) {
    commands.spawn_empty().spawn_task(move |cmd| async move {
        ctrl_c().await.ok();
        cmd.send_state(Message::RequestShutdown);
    });
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn build_server_app(iso: IsomorphicApp<Message, crate::Command>) -> (App, ecsdk::app::Receivers<Message>) {
    let containers = [
        ("postgres", "postgres:16", 0),
        ("redis", "redis:7", 0),
        ("api-server", "myapp/api:latest", 1),
        ("web-frontend", "myapp/web:latest", 2),
    ];

    let mut app = iso.build_server(crate::Command::Up);

    let wake = app.world().resource::<WakeSignal>().clone();
    let (tracing_layer, tracing_receiver) = ecsdk::tracing::setup(wake);
    tracing_subscriber::registry()
        .with(tracing_layer.with_filter(
            tracing_subscriber::filter::Targets::new().with_target("compose", tracing::Level::INFO),
        ))
        .init();
    app.add_plugins(ecsdk::tracing::TracingPlugin::new(tracing_receiver));

    app.add_plugins(DaemonPlugin);

    let state_queue = app.world().resource::<MessageQueue<Message>>().clone();
    for (name, image, order) in containers {
        state_queue.send(Message::SpawnContainer {
            name: name.into(),
            image: image.into(),
            start_order: order,
        });
    }

    let mut orchestrator = app.world_mut().spawn((build_orchestrator_sm(),));
    OrchestratorPhase::Deploying.insert_marker_world(&mut orchestrator);

    app.world_mut().spawn((
        ContainerName("[system]".into()),
        LogBuffer::default(),
        SystemEntity,
        Replicated,
    ));

    // Backend factory — attach backends to containers spawned by state events
    app.add_observer(attach_mock_backend);

    app.into_parts()
}

pub async fn run_daemon(iso: IsomorphicApp<Message, crate::Command>) {
    let (mut app, rx) = build_server_app(iso);
    ecsdk::app::run_async(&mut app, rx).await;

    let _ = std::fs::remove_file(crate::ipc::SOCKET_PATH);
    tracing::info!("Daemon shut down");
}

fn attach_mock_backend(
    trigger: On<Add, ContainerName>,
    mut commands: Commands,
    query: Query<(&ContainerName, &ImageRef), Without<Backend>>,
) {
    let entity = trigger.event_target();
    let Ok((name, image)) = query.get(entity) else {
        return;
    };
    commands
        .entity(entity)
        .insert((Backend(MockBackend::new(&name.0, &image.0)), Replicated));
}
