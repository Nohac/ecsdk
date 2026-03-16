use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy::state::prelude::*;
use bevy_replicon::prelude::*;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ecsdk_core::{AppExit, WakeSignal};
use ecsdk_term::TerminalEvent;
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::container::*;
use crate::message::Message;
use crate::protocol::{LogEvent, ServerExitNotice, ShutdownRequest, StatusRequest};
use crate::render::{CrosstermPlugin, RenderMode};
use crate::replicon::{SharedReplicationPlugin, spawn_client_connection};

// ---------------------------------------------------------------------------
// Client-specific observers and systems
// ---------------------------------------------------------------------------

fn on_remote_added(trigger: On<Add, Remote>, mut commands: Commands) {
    commands
        .entity(trigger.event_target())
        .insert(LogBuffer::default());
}

fn on_log_event(trigger: On<LogEvent>, mut logs: Query<&mut LogBuffer>) {
    let event = trigger.event();
    if let Ok(mut log_buf) = logs.get_mut(event.container_entity) {
        log_buf.push(&event.text);
    }
}

fn on_server_exit(_trigger: On<ServerExitNotice>, mut exit: ResMut<AppExit>) {
    exit.0 = true;
}

fn on_ctrl_c(trigger: On<TerminalEvent>, mut commands: Commands) {
    if let Event::Key(key) = &trigger.event().0
        && key.code == KeyCode::Char('c')
        && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        commands.client_trigger(ShutdownRequest);
    }
}

fn detect_disconnect(
    state: Res<State<ClientState>>,
    mut exit: ResMut<AppExit>,
    mut was_connected: Local<bool>,
) {
    if *state.get() == ClientState::Connected {
        *was_connected = true;
    } else if *was_connected {
        exit.0 = true;
    }
}

// ---------------------------------------------------------------------------
// ClientPlugin — bundles all client-side registration
// ---------------------------------------------------------------------------

pub struct ClientPlugin(pub RenderMode);

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        // Infrastructure plugins required by replicon
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.add_plugins(bevy::time::TimePlugin);

        // Replicon client
        app.add_plugins(RepliconPlugins);
        app.add_plugins(SharedReplicationPlugin);
        app.add_plugins(ecsdk_replicon::ClientTransportPlugin);
        app.add_systems(Startup, spawn_client_connection);

        // Tracing → LogBuffer drain
        app.add_systems(
            PreUpdate,
            crate::container::drain_tracing_logs
                .run_if(resource_exists::<ecsdk_tracing::TracingReceiver>),
        );

        // Rendering
        app.add_plugins(CrosstermPlugin::new(self.0));
        app.init_resource::<MergedLogView>();

        // Observers
        app.add_observer(on_remote_added);
        app.add_observer(on_log_event);
        app.add_observer(on_server_exit);
        app.add_observer(on_ctrl_c);

        // Disconnect detection
        app.add_systems(Update, detect_disconnect);
    }
}

pub struct StatusPlugin;

impl Plugin for StatusPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.add_plugins(bevy::time::TimePlugin);

        // Replicon client
        app.add_plugins(RepliconPlugins);
        app.add_plugins(SharedReplicationPlugin);
        app.add_plugins(ecsdk_replicon::ClientTransportPlugin);
        app.add_systems(Startup, spawn_client_connection);

        // Observers
        app.add_observer(send_status_request_on_initial_connection);
        app.add_observer(handle_status_response);

        // Disconnect detection
        app.add_systems(Update, detect_disconnect);
    }
}

fn send_status_request_on_initial_connection(
    _trigger: On<Add, InitialConnection>,
    mut commands: Commands,
) {
    commands.client_trigger(StatusRequest);
}

fn handle_status_response(trigger: On<crate::protocol::StatusResponse>, mut exit: ResMut<AppExit>) {
    let e = trigger.event();
    println!("time: {:?}", e.time);
    println!("note: {}", e.note);
    exit.0 = true;
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn run_client(mode: RenderMode, com: &super::Command) {
    let (mut app, rx) = ecsdk_app::setup::<Message>();

    let wake = app.world().resource::<WakeSignal>().clone();
    let (tracing_layer, tracing_receiver) = ecsdk_tracing::setup(wake);
    tracing_subscriber::registry()
        .with(tracing_layer.with_filter(
            tracing_subscriber::filter::Targets::new().with_target("compose", tracing::Level::INFO),
        ))
        .init();
    app.add_plugins(ecsdk_tracing::TracingPlugin::new(tracing_receiver));

    match com {
        crate::Command::Up => app.add_plugins(ClientPlugin(mode)),
        crate::Command::Status => app.add_plugins(StatusPlugin),
    };
    ecsdk_app::run_async(&mut app, rx).await;
}
