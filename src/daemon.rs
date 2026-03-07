use std::collections::HashMap;

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy_replicon::prelude::*;
use tokio::signal::ctrl_c;

use crate::backend_mock::MockBackend;
use crate::container::*;
use crate::lifecycle::*;
use crate::msg::{AppExit, TriggerEvent};
use crate::protocol::{LogEvent, ServerExitNotice, ShutdownRequest};
use crate::replicon_transport::*;
use crate::task::SpawnTask;

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
                message: LogEvent {
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
            message: ServerExitNotice,
        });
        *sent = true;
    }
}

fn handle_shutdown_request(
    _trigger: On<FromClient<ShutdownRequest>>,
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
        app.add_plugins(
            RepliconPlugins
                .build()
                .set(ServerPlugin::new(PostUpdate)),
        );
        app.add_plugins(SharedReplicationPlugin);
        app.add_plugins(ServerTransportPlugin);

        // Lifecycle + log/exit broadcast
        app.add_plugins(LifecyclePlugin);
        app.add_systems(Update, (send_log_events, send_exit_notice));
        app.add_observer(handle_shutdown_request);

        // Ctrl+C triggers graceful shutdown
        app.add_systems(Startup, spawn_ctrl_c_handler);
    }
}

fn spawn_ctrl_c_handler(mut commands: Commands) {
    commands.spawn_empty().spawn_task(move |cmd| async move {
        ctrl_c().await.ok();
        cmd.send(TriggerEvent(ShutdownAll));
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

    let (mut app, msg_rx) = crate::app::setup();
    app.add_plugins(DaemonPlugin);

    for (name, image, order) in containers {
        app.world_mut().spawn((
            ContainerName(name.into()),
            ImageRef(image.into()),
            StartOrder(order),
            ContainerPhase::Pending,
            Pending,
            build_container_sm(),
            LogBuffer::default(),
            Backend(MockBackend::new(name, image)),
            Replicated,
        ));
    }

    app.world_mut().spawn((
        Deploying,
        build_orchestrator_sm(),
    ));

    app.world_mut().spawn((
        ContainerName("[system]".into()),
        LogBuffer::default(),
        SystemEntity,
        Replicated,
    ));

    crate::app::run_async(app, msg_rx).await;

    let _ = std::fs::remove_file(crate::ipc::SOCKET_PATH);
    eprintln!("Daemon shut down");
}
