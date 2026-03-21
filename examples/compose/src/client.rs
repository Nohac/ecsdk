use bevy::state::prelude::*;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ecsdk::prelude::*;
use ecsdk::core::{AppExit, WakeSignal};
use ecsdk::term::TerminalEvent;
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

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
    pub command: crate::Command,
}

impl Plugin for ComposeClientPlugin {
    fn build(&self, app: &mut App) {
        // Tracing → LogBuffer drain
        app.add_systems(
            PreUpdate,
            crate::container::drain_tracing_logs
                .run_if(resource_exists::<ecsdk::tracing::TracingReceiver>),
        );

        match self.command {
            crate::Command::Up => {
                app.add_plugins(CrosstermPlugin::new(self.mode));
                app.init_resource::<MergedLogView>();
                app.add_observer(on_remote_added);
                app.add_observer(on_log_event);
                app.add_observer(on_server_exit);
                app.add_observer(on_ctrl_c);
            }
            crate::Command::Status => {}
        }

        app.add_systems(Update, detect_disconnect);
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn run_client(iso: IsomorphicApp<Message>, mode: RenderMode, com: crate::Command) {
    let mut app = iso.build_client();

    let wake = app.world().resource::<WakeSignal>().clone();
    let (tracing_layer, tracing_receiver) = ecsdk::tracing::setup(wake);
    tracing_subscriber::registry()
        .with(tracing_layer.with_filter(
            tracing_subscriber::filter::Targets::new().with_target("compose", tracing::Level::INFO),
        ))
        .init();
    app.add_plugins(ecsdk::tracing::TracingPlugin::new(tracing_receiver));
    app.add_plugins(ComposeClientPlugin { mode, command: com });
    if matches!(com, crate::Command::Status) {
        StatusFeature::register_client(&mut app);
    }

    let (mut app, rx) = app.into_parts();
    ecsdk::app::run_async(&mut app, rx).await;
}
