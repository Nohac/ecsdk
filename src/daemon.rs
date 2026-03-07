use std::collections::HashMap;

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy_replicon::prelude::*;
use tokio::sync::mpsc;
use tokio::signal::ctrl_c;

use interprocess::local_socket::traits::tokio::Listener as _;

use crate::backend_mock::MockBackend;
use crate::bridge::AppExit;
use crate::container::*;
use crate::lifecycle::*;
use crate::protocol::{LogEvent, ServerExitNotice, ShutdownRequest};
use crate::replicon_transport::*;
use crate::task::CommandSender;

/// ECS system that diffs LogBuffer line counts per entity and sends new lines
/// as LogEvent via replicon server events.
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

/// ECS system that sends ServerExitNotice once when AppExit becomes true.
fn send_exit_notice(mut commands: Commands, exit: Res<AppExit>, mut sent: Local<bool>) {
    if exit.0 && !*sent {
        commands.server_trigger(ToClients {
            mode: SendMode::Broadcast,
            message: ServerExitNotice,
        });
        *sent = true;
    }
}

/// Observer: client requests shutdown → trigger ShutdownAll.
fn handle_shutdown_request(
    _trigger: On<FromClient<ShutdownRequest>>,
    mut commands: Commands,
) {
    commands.trigger(ShutdownAll);
}

/// Run the daemon process: ECS world + replicon server + IPC on a local socket.
pub async fn run_daemon() {
    let containers = [
        ("postgres", "postgres:16", 0),
        ("redis", "redis:7", 0),
        ("api-server", "myapp/api:latest", 1),
        ("web-frontend", "myapp/web:latest", 2),
    ];

    let (mut app, cmd_rx) = crate::app::setup();

    // Infrastructure plugins required by replicon
    app.add_plugins(bevy::state::app::StatesPlugin);
    app.add_plugins(bevy::time::TimePlugin);

    // Replicon plugins — tick every frame via PostUpdate
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

    // Spawn containers with Replicated marker
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

    // Orchestrator entity
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

    let cmd_sender = app.world().resource::<CommandSender>().clone();

    // IPC listener
    let listener = crate::ipc::create_listener().expect("Failed to bind daemon socket");
    eprintln!("Daemon listening on {}", crate::ipc::SOCKET_PATH);

    let accept_sender = cmd_sender.clone();
    let accept_loop = async move {
        loop {
            let stream = match listener.accept().await {
                Ok(stream) => stream,
                Err(e) => {
                    eprintln!("Accept failed: {e}");
                    continue;
                }
            };

            let cmd_tx = accept_sender.clone();

            // Per-connection handler
            tokio::spawn(async move {
                // Create mpsc channels for bridging stream ↔ ECS
                let (to_client_tx, mut to_client_rx) = mpsc::unbounded_channel::<RepliconPacket>();
                let (from_client_tx, from_client_rx) = mpsc::unbounded_channel::<RepliconPacket>();

                // Spawn a ConnectedClient entity and register in the ServerBridge
                let client_entity = cmd_tx
                    .query(|world: &mut World| {
                        let client = world
                            .spawn(ConnectedClient { max_size: 1200 })
                            .id();
                        world.resource_mut::<ServerBridge>().clients.insert(
                            client,
                            ServerClientChannels {
                                from_client_rx,
                                to_client_tx,
                            },
                        );
                        client
                    })
                    .await;

                let wake_tx = cmd_tx.clone();
                run_bridge(stream, &mut to_client_rx, &from_client_tx, move || {
                    wake_tx.send(|_: &mut World| {});
                })
                .await;

                // Clean up on disconnect
                cmd_tx.send(move |world: &mut World| {
                    world
                        .resource_mut::<ServerBridge>()
                        .clients
                        .remove(&client_entity);
                    if world.get_entity(client_entity).is_ok() {
                        world.despawn(client_entity);
                    }
                });
            });
        }
    };

    // Ctrl+C triggers graceful shutdown (doesn't cancel run_async)
    tokio::spawn(async move {
        ctrl_c().await.ok();
        cmd_sender.trigger(ShutdownAll);
    });

    tokio::select! {
        _ = crate::app::run_async(app, cmd_rx) => {}
        _ = accept_loop => {}
    }

    // Cleanup
    let _ = std::fs::remove_file(crate::ipc::SOCKET_PATH);
    eprintln!("Daemon shut down");
}
