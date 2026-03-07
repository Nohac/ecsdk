use bevy::ecs::prelude::*;
use bevy::state::prelude::*;
use bevy_replicon::prelude::*;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use roam_stream::{HandshakeConfig, NoDispatcher, connect};
use tokio::sync::mpsc;

use crate::bridge::AppExit;
use crate::container::*;
use crate::ipc::{DaemonConnector, SOCKET_PATH};
use crate::protocol::{LogEvent, ServerExitNotice, ShutdownRequest};
use crate::render::{CrosstermPlugin, RenderMode, TerminalEvent};
use crate::replicon_transport::*;
use crate::task::CommandSender;

/// Observer: when replicon creates a new entity from the server, insert a LogBuffer.
fn on_remote_added(trigger: On<Add, Remote>, mut commands: Commands) {
    commands.entity(trigger.event_target()).insert(LogBuffer::default());
}

/// Observer: append log text to the entity's LogBuffer.
fn on_log_event(trigger: On<LogEvent>, mut logs: Query<&mut LogBuffer>) {
    let event = trigger.event();
    if let Ok(mut log_buf) = logs.get_mut(event.container_entity) {
        log_buf.push(&event.text);
    }
}

/// Observer: server is exiting — set AppExit.
fn on_server_exit(_trigger: On<ServerExitNotice>, mut exit: ResMut<AppExit>) {
    exit.0 = true;
}

/// Observer: Ctrl+C → send ShutdownRequest to server.
fn on_ctrl_c(trigger: On<TerminalEvent>, mut commands: Commands) {
    if let Event::Key(key) = &trigger.event().0
        && key.code == KeyCode::Char('c')
        && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        commands.client_trigger(ShutdownRequest);
    }
}

/// Run the client: connect to daemon via replicon, render.
pub async fn run_client(mode: RenderMode) {
    let (mut app, cmd_rx) = crate::app::setup();

    // Infrastructure plugins required by replicon
    app.add_plugins(bevy::state::app::StatesPlugin);
    app.add_plugins(bevy::time::TimePlugin);

    // Replicon plugins
    app.add_plugins(RepliconPlugins);
    app.add_plugins(SharedReplicationPlugin);
    app.add_plugins(RoamClientPlugin);

    // Rendering
    app.add_plugins(CrosstermPlugin::new(mode));
    app.init_resource::<MergedLogView>();

    // Observers
    app.add_observer(on_remote_added);
    app.add_observer(on_log_event);
    app.add_observer(on_server_exit);
    app.add_observer(on_ctrl_c);

    // Exit when disconnected after having been connected (fallback)
    app.add_systems(bevy::app::Update, |state: Res<State<ClientState>>,
                             mut exit: ResMut<AppExit>,
                             mut was_connected: Local<bool>| {
        if *state.get() == ClientState::Connected {
            *was_connected = true;
        } else if *was_connected {
            exit.0 = true;
        }
    });

    let cmd_sender = app.world().resource::<CommandSender>().clone();

    // Async task: connect via roam, create bridge, forward packets
    let connect_sender = cmd_sender.clone();
    let connection = async move {
        let connector = DaemonConnector::new(SOCKET_PATH);
        let roam_client = connect(connector, HandshakeConfig::default(), NoDispatcher);
        let client = RepliconTransportClient::new(roam_client);

        // Create mpsc channels for bridging roam ↔ ECS
        let (to_server_tx, mut to_server_rx) = mpsc::unbounded_channel::<RepliconPacket>();
        let (from_server_tx, from_server_rx) = mpsc::unbounded_channel::<RepliconPacket>();

        // Create roam channel pairs for bidirectional streaming:
        // to_client: server writes → client reads
        let (to_client_tx, mut to_client_rx) = roam::channel::<RepliconPacket>();
        // from_client: client writes → server reads
        let (from_client_tx, from_client_rx) = roam::channel::<RepliconPacket>();

        // Insert the ClientBridge resource
        connect_sender.send(move |world: &mut World| {
            world.insert_resource(ClientBridge {
                from_server_rx,
                to_server_tx,
            });
        });

        // Call replicate() — passes Tx for server→client, Rx for client→server
        let replicate_fut = async {
            let _ = client.replicate(to_client_tx, from_client_rx).await;
        };

        // Forward: roam to_client_rx → mpsc from_server_tx (server→client→ECS)
        // Send a no-op command to wake the event loop so app.update() runs.
        let wake_sender = connect_sender.clone();
        let forward_from_server = async {
            while let Ok(Some(packet)) = to_client_rx.recv().await {
                let _ = from_server_tx.send(packet);
                wake_sender.send(|_: &mut World| {});
            }
        };

        // Forward: mpsc to_server_rx → roam from_client_tx (ECS→client→server)
        let forward_to_server = async {
            while let Some(packet) = to_server_rx.recv().await {
                if from_client_tx.send(&packet).await.is_err() {
                    break;
                }
            }
        };

        tokio::select! {
            _ = replicate_fut => {}
            _ = forward_from_server => {}
            _ = forward_to_server => {}
        }

        // Disconnected — signal exit
        connect_sender.send(|world: &mut World| {
            world.remove_resource::<ClientBridge>();
            world.resource_mut::<AppExit>().0 = true;
        });
    };

    tokio::select! {
        _ = crate::app::run_async(app, cmd_rx) => {}
        _ = connection => {}
    }
}
