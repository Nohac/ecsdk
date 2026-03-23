use bevy::state::prelude::*;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ecsdk::prelude::*;
use ecsdk::term::TerminalEvent;

use crate::message::Message;
use crate::protocol::{ServerExitNotice, ShutdownRequest};
use crate::render::{CrosstermPlugin, RenderMode};
use crate::status::StatusFeature;

// ---------------------------------------------------------------------------
// Client-specific observers and systems
// ---------------------------------------------------------------------------

fn on_server_exit(_trigger: On<ServerExitNotice>, mut exit: MessageWriter<AppExit>) {
    exit.write(AppExit::Success);
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
    mut exit: MessageWriter<AppExit>,
    mut was_connected: Local<bool>,
) {
    if *state.get() == ClientState::Connected {
        *was_connected = true;
    } else if *was_connected {
        exit.write(AppExit::Success);
    }
}

// ---------------------------------------------------------------------------
// ClientPlugin — bundles all client-side registration
// ---------------------------------------------------------------------------

pub struct ComposeClientPlugin {
    pub mode: RenderMode,
}

impl Plugin for ComposeClientPlugin {
    fn build(&self, app: &mut App) {
        // Tracing → LogBuffer drain
        app.add_systems(
            PreUpdate,
            crate::container::drain_tracing_logs
                .run_if(resource_exists::<ecsdk::tracing::TracingReceiver>),
        );

        app.add_plugins(CrosstermPlugin::new(self.mode));
        app.add_observer(on_server_exit);
        app.add_observer(on_ctrl_c);

        app.add_systems(Update, detect_disconnect);
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run_up(
    mut app: AsyncApp<Message>,
    mode: RenderMode,
) -> AsyncApp<Message> {
    app.add_plugins(ComposeClientPlugin { mode });
    app
}

pub fn run_status(mut app: AsyncApp<Message>) -> AsyncApp<Message> {
    StatusFeature::register_client(&mut app);
    app
}
