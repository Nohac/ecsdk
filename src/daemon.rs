use std::collections::HashMap;

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy_replicon::prelude::*;
use roam_stream::{HandshakeConfig, accept};
use tokio::sync::mpsc;
use tokio::net::UnixListener;
use tokio::signal::ctrl_c;

use crate::app::TaskQueue;
use crate::backend_mock::MockBackend;
use crate::bridge::AppExit;
use crate::container::*;
use crate::ipc::SOCKET_PATH;
use crate::lifecycle::{Backend, LifecyclePlugin, ShutdownAll};
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

/// Roam service implementation for replicon transport.
#[derive(Clone)]
struct TransportImpl {
    cmd_tx: CommandSender,
}

impl RepliconTransport for TransportImpl {
    async fn replicate(
        &self,
        _cx: &roam::Context,
        to_client: roam::Tx<RepliconPacket>,
        mut from_client: roam::Rx<RepliconPacket>,
    ) {
        // Create mpsc channels for bridging roam ↔ ECS
        let (to_client_tx, mut to_client_rx) = mpsc::unbounded_channel::<RepliconPacket>();
        let (from_client_tx, from_client_rx) = mpsc::unbounded_channel::<RepliconPacket>();

        // Spawn a ConnectedClient entity and register in the ServerBridge
        let cmd_tx = self.cmd_tx.clone();
        let client_entity = cmd_tx
            .query(|world: &mut World| {
                let client = world
                    .spawn(ConnectedClient {
                        max_size: 1200,
                    })
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

        // Bidirectional forwarding: roam ↔ mpsc
        let forward_to_client = async {
            while let Some(packet) = to_client_rx.recv().await {
                if to_client.send(&packet).await.is_err() {
                    break;
                }
            }
        };

        let forward_from_client = async {
            while let Ok(Some(packet)) = from_client.recv().await {
                let _ = from_client_tx.send(packet);
            }
        };

        tokio::select! {
            _ = forward_to_client => {}
            _ = forward_from_client => {}
        }

        // Clean up on disconnect
        cmd_tx.send(move |world: &mut World| {
            world.resource_mut::<ServerBridge>().clients.remove(&client_entity);
            if world.get_entity(client_entity).is_ok() {
                world.despawn(client_entity);
            }
        });
    }

    async fn ping(&self, _cx: &roam::Context) -> Result<String, String> {
        Ok("pong".into())
    }
}

/// Run the daemon process: ECS world + replicon server + IPC on a Unix socket.
pub async fn run_daemon() {
    // Clean up stale socket
    let _ = std::fs::remove_file(SOCKET_PATH);

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
    app.add_plugins(RoamServerPlugin);

    // Lifecycle + log/exit broadcast
    app.add_plugins(LifecyclePlugin);
    app.add_systems(
        Update,
        (send_log_events, send_exit_notice)
            .after(crate::lifecycle::enforce_ordering)
            .after(crate::lifecycle::check_all_running)
            .after(crate::lifecycle::check_all_stopped),
    );
    app.add_observer(handle_shutdown_request);

    // Spawn containers with Replicated marker
    for (name, image, order) in containers {
        app.world_mut().spawn((
            ContainerName(name.into()),
            ImageRef(image.into()),
            StartOrder(order),
            ContainerPhase::Pending,
            LogBuffer::default(),
            Backend(MockBackend::new(name, image)),
            Replicated,
        ));
    }
    app.world_mut().spawn((
        ContainerName("[system]".into()),
        LogBuffer::default(),
        SystemEntity,
        Replicated,
    ));

    let cmd_sender = app.world().resource::<CommandSender>().clone();
    let handler = TransportImpl {
        cmd_tx: cmd_sender.clone(),
    };

    // IPC listener
    let listener = UnixListener::bind(SOCKET_PATH).expect("Failed to bind daemon socket");
    eprintln!("Daemon listening on {SOCKET_PATH}");

    let accept_handler = handler.clone();
    {
        let mut tasks = app.world_mut().resource_mut::<TaskQueue>();
        tasks.push(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let dispatcher = RepliconTransportDispatcher::new(accept_handler.clone());
                // Per-connection handler — genuine concurrency, spawn is correct
                tokio::spawn(async move {
                    match accept(stream, HandshakeConfig::default(), dispatcher).await {
                        Ok((_handle, _incoming, driver)) => {
                            let _ = driver.run().await;
                        }
                        Err(e) => eprintln!("Handshake failed: {e}"),
                    }
                });
            }
        });

        // Ctrl+C
        tasks.push(async move {
            ctrl_c().await.ok();
            cmd_sender.trigger(ShutdownAll);
        });
    }

    crate::app::run_async(app, cmd_rx).await;

    // Cleanup
    let _ = std::fs::remove_file(SOCKET_PATH);
    eprintln!("Daemon shut down");
}
