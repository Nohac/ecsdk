use std::collections::HashMap;

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy_replicon::prelude::*;
use ecsdk_core::{AppExit, MessageQueue, WakeSignal};
use ecsdk_tasks::SpawnTask;
use tokio::signal::ctrl_c;
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::backend_mock::MockBackend;
use crate::container::*;
use crate::lifecycle::*;
use crate::message::Message;
use crate::replicon::{SharedReplicationPlugin, spawn_server_listener};

// ---------------------------------------------------------------------------
// Daemon-specific ECS systems and observers
// ---------------------------------------------------------------------------

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
        // Infrastructure plugins required by replicon
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.add_plugins(bevy::time::TimePlugin);

        // Replicon server
        app.add_plugins(RepliconPlugins.build().set(ServerPlugin::new(PostUpdate)));
        app.add_plugins(SharedReplicationPlugin);
        app.add_plugins(ecsdk_replicon::ServerTransportPlugin);
        app.add_systems(Startup, spawn_server_listener);

        // Lifecycle + log/exit broadcast
        app.add_plugins(LifecyclePlugin);
        app.add_systems(
            PreUpdate,
            crate::container::drain_tracing_logs
                .run_if(resource_exists::<ecsdk_tracing::TracingReceiver>),
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

pub async fn run_daemon() {
    let containers = [
        ("postgres", "postgres:16", 0),
        ("redis", "redis:7", 0),
        ("api-server", "myapp/api:latest", 1),
        ("web-frontend", "myapp/web:latest", 2),
    ];

    let (mut app, rx) = ecsdk_app::setup::<Message>();

    let wake = app.world().resource::<WakeSignal>().clone();
    let (tracing_layer, tracing_receiver) = ecsdk_tracing::setup(wake);
    tracing_subscriber::registry()
        .with(tracing_layer.with_filter(
            tracing_subscriber::filter::Targets::new()
                .with_target("compose", tracing::Level::INFO),
        ))
        .init();
    app.add_plugins(ecsdk_tracing::TracingPlugin::new(tracing_receiver));

    app.add_plugins(DaemonPlugin);

    let state_queue = app.world().resource::<MessageQueue<Message>>().clone();
    for (name, image, order) in containers {
        state_queue.send(Message::SpawnContainer {
            name: name.into(),
            image: image.into(),
            start_order: order,
        });
    }

    app.world_mut().spawn((Deploying, build_orchestrator_sm()));

    app.world_mut().spawn((
        ContainerName("[system]".into()),
        LogBuffer::default(),
        SystemEntity,
        Replicated,
    ));

    // Backend factory — attach backends to containers spawned by state events
    app.add_observer(attach_mock_backend);

    ecsdk_app::run_async(app, rx).await;

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
