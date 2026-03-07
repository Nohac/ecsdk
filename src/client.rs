use bevy::ecs::prelude::*;
use bevy::state::prelude::*;
use bevy_replicon::prelude::*;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use tokio::sync::mpsc;

use crate::bridge::AppExit;
use crate::container::*;
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
    app.add_plugins(ClientTransportPlugin);

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

    // Async task: connect via local socket, bridge packets
    let connect_sender = cmd_sender.clone();
    let connection = async move {
        let stream = match crate::ipc::connect().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to connect to daemon: {e}");
                connect_sender.send(|world: &mut World| {
                    world.resource_mut::<AppExit>().0 = true;
                });
                return;
            }
        };

        // Create mpsc channels for bridging stream ↔ ECS
        let (to_server_tx, mut to_server_rx) = mpsc::unbounded_channel::<RepliconPacket>();
        let (from_server_tx, from_server_rx) = mpsc::unbounded_channel::<RepliconPacket>();

        // Insert the ClientBridge resource
        connect_sender.send(move |world: &mut World| {
            world.insert_resource(ClientBridge {
                from_server_rx,
                to_server_tx,
            });
        });

        let wake_sender = connect_sender.clone();
        run_bridge(stream, &mut to_server_rx, &from_server_tx, move || {
            wake_sender.send(|_: &mut World| {});
        })
        .await;

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
