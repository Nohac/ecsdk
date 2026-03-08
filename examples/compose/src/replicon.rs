use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy_replicon::prelude::*;
use ecsdk_core::AppExit;
use ecsdk_replicon::{
    AcceptClientCmd, InsertClientBridgeCmd, RemoveClientBridgeCmd, RepliconPacket, run_bridge,
};
use ecsdk_tasks::SpawnCmdTask;
use interprocess::local_socket::traits::tokio::Listener as _;
use tokio::sync::mpsc;

use crate::container::*;
use crate::protocol::{LogEvent, ServerExitNotice, ShutdownRequest};

// ---------------------------------------------------------------------------
// Shared replication plugin — ensures identical registration order
// ---------------------------------------------------------------------------

pub struct SharedReplicationPlugin;

impl Plugin for SharedReplicationPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<ContainerName>();
        app.replicate::<ImageRef>();
        app.replicate::<StartOrder>();
        app.replicate::<ContainerPhase>();
        app.replicate::<DownloadProgress>();
        app.replicate::<SystemEntity>();

        app.add_mapped_server_event::<LogEvent>(Channel::Ordered);
        app.add_server_event::<ServerExitNotice>(Channel::Ordered);
        app.add_client_event::<ShutdownRequest>(Channel::Ordered);
    }
}

// ---------------------------------------------------------------------------
// Server listener (app-specific: uses interprocess)
// ---------------------------------------------------------------------------

pub fn spawn_server_listener(mut commands: Commands) {
    commands
        .spawn_empty()
        .spawn_cmd_task(move |cmd| async move {
            let listener = crate::ipc::create_listener().expect("Failed to bind daemon socket");
            eprintln!("Daemon listening on {}", crate::ipc::SOCKET_PATH);

            loop {
                let stream = match listener.accept().await {
                    Ok(stream) => stream,
                    Err(e) => {
                        eprintln!("Accept failed: {e}");
                        continue;
                    }
                };

                cmd.send(move |world: &mut World| {
                    AcceptClientCmd { stream }.apply(world);
                })
                .wake();
            }
        });
}

// ---------------------------------------------------------------------------
// Client connection (app-specific: uses interprocess)
// ---------------------------------------------------------------------------

pub fn spawn_client_connection(mut commands: Commands) {
    commands
        .spawn_empty()
        .spawn_cmd_task(move |cmd| async move {
            let stream = match crate::ipc::connect().await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to connect to daemon: {e}");
                    cmd.send(|world: &mut World| {
                        world.resource_mut::<AppExit>().0 = true;
                    })
                    .wake();
                    return;
                }
            };

            let (to_server_tx, mut to_server_rx) = mpsc::unbounded_channel::<RepliconPacket>();
            let (from_server_tx, from_server_rx) = mpsc::unbounded_channel::<RepliconPacket>();

            cmd.send(move |world: &mut World| {
                InsertClientBridgeCmd {
                    from_server_rx,
                    to_server_tx,
                }
                .apply(world);
            })
            .wake();

            let wake = cmd.clone();
            run_bridge(stream, &mut to_server_rx, &from_server_tx, move || {
                wake.wake();
            })
            .await;

            cmd.send(|world: &mut World| {
                RemoveClientBridgeCmd.apply(world);
                world.resource_mut::<AppExit>().0 = true;
            })
            .wake();
        });
}
