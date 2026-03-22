use ecsdk::prelude::*;
use ecsdk::tasks::SpawnCmdTask;
use interprocess::local_socket::traits::tokio::Listener as _;

use crate::container::*;
use crate::protocol::{ServerExitNotice, ShutdownRequest};

// ---------------------------------------------------------------------------
// Shared replication plugin — ensures identical registration order
// ---------------------------------------------------------------------------

pub struct SharedReplicationPlugin;

impl IsomorphicPlugin for SharedReplicationPlugin {
    fn build_shared(&self, app: &mut App) {
        app.replicate::<ContainerName>();
        app.replicate::<ImageRef>();
        app.replicate::<StartOrder>();
        app.replicate::<DownloadProgress>();
        app.replicate::<SystemEntity>();
        app.replicate::<LogEntry>();

        ContainerPhase::replicate_markers(app);
        OrchestratorPhase::replicate_markers(app);

        app.add_server_event::<ServerExitNotice>(Channel::Ordered);
        app.add_client_event::<ShutdownRequest>(Channel::Ordered);
    }
}

pub struct ConnectionPlugin;

impl IsomorphicPlugin for ConnectionPlugin {
    fn build_server(&self, app: &mut App) {
        app.add_systems(Startup, spawn_server_listener);
    }

    fn build_client(&self, app: &mut App) {
        app.add_systems(Startup, spawn_client_connection);
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
            tracing::warn!("Daemon listening on {}", crate::ipc::SOCKET_PATH);

            loop {
                let stream = match listener.accept().await {
                    Ok(stream) => stream,
                    Err(e) => {
                        tracing::warn!("Accept failed: {e}");
                        continue;
                    }
                };

                cmd.send(move |world: &mut World| {
                    ecsdk::network::AcceptClientCmd { stream }.apply(world);
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
            match crate::ipc::connect().await {
                Ok(stream) => {
                    cmd.send(move |world: &mut World| {
                        ecsdk::network::ConnectClientCmd { stream }.apply(world);
                    })
                    .wake();
                }
                Err(e) => {
                    tracing::warn!("Failed to connect to daemon: {e}");
                    cmd.send(|world: &mut World| {
                        world.resource_mut::<AppExit>().0 = true;
                    })
                    .wake();
                }
            }
        });
}
