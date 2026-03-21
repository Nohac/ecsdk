use bevy::state::prelude::*;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ecsdk::core::AppExit;
use ecsdk::prelude::*;
use ecsdk::term::TerminalEvent;

use crate::container::*;
use crate::message::Message;
use crate::protocol::{LogEvent, ServerExitNotice, ShutdownRequest};
use crate::render::{CrosstermPlugin, RenderMode};
use crate::status::StatusFeature;

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
        app.init_resource::<MergedLogView>();
        app.add_observer(on_remote_added);
        app.add_observer(on_log_event);
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
