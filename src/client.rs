use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy::state::prelude::*;
use bevy_replicon::prelude::*;
use crossterm::event::{Event, KeyCode, KeyModifiers};

use crate::container::*;
use crate::msg::AppExit;
use crate::protocol::{LogEvent, ServerExitNotice, ShutdownRequest};
use crate::render::{CrosstermPlugin, RenderMode, TerminalEvent};
use crate::replicon_transport::*;

// ---------------------------------------------------------------------------
// Client-specific observers and systems
// ---------------------------------------------------------------------------

fn on_remote_added(trigger: On<Add, Remote>, mut commands: Commands) {
    commands.entity(trigger.event_target()).insert(LogBuffer::default());
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
        app.add_plugins(ClientTransportPlugin);

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

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn run_client(mode: RenderMode) {
    let (mut app, rx) = crate::app::setup();
    app.add_plugins(ClientPlugin(mode));
    crate::app::run_async(app, rx).await;
}
