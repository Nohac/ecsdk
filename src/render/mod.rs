mod plain;
mod tui;

pub use tui::TerminalGuard;

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use clap::ValueEnum;
use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;

use crate::container::build_merged_log_view;
use crate::msg::{Msg, TriggerEvent};
use crate::task::SpawnTask;

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum RenderMode {
    Plain,
    Tui,
}

/// Current terminal dimensions, kept up-to-date via resize events.
#[derive(Resource)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    pub fn query_now() -> Self {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        Self { cols, rows }
    }
}

/// Terminal event forwarded into the ECS as a bevy Event.
#[derive(Event)]
pub struct TerminalEvent(pub Event);

/// Marker component for the crossterm event loop entity.
#[derive(Component)]
struct CrosstermEntity;

// ── Render messages ──

pub enum RenderMsg {
    Resize(ResizeCmd),
}

impl Msg for RenderMsg {
    fn apply(self: Box<Self>, commands: &mut Commands) {
        match *self {
            Self::Resize(cmd) => commands.queue(cmd),
        }
    }
}

pub struct ResizeCmd {
    pub cols: u16,
    pub rows: u16,
}

impl Command for ResizeCmd {
    fn apply(self, world: &mut World) {
        let mut size = world.resource_mut::<TerminalSize>();
        size.cols = self.cols;
        size.rows = self.rows;
    }
}

pub struct CrosstermPlugin {
    mode: RenderMode,
}

impl CrosstermPlugin {
    pub fn new(mode: RenderMode) -> Self {
        Self { mode }
    }
}

impl Plugin for CrosstermPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, build_merged_log_view);
        match self.mode {
            RenderMode::Tui => {
                app.insert_resource(TerminalGuard::new());
                app.insert_resource(TerminalSize::query_now());
                app.add_systems(Startup, setup_crossterm);
                app.add_systems(PostUpdate, tui::render_tui);
            }
            RenderMode::Plain => {
                app.add_systems(PostUpdate, plain::render_plain);
            }
        }
    }
}

fn setup_crossterm(mut commands: Commands) {
    commands
        .spawn(CrosstermEntity)
        .spawn_task(|cmd| async move {
            let mut events = EventStream::new();
            while let Some(Ok(event)) = events.next().await {
                if let Event::Resize(cols, rows) = event {
                    cmd.send(RenderMsg::Resize(ResizeCmd { cols, rows }));
                }
                cmd.send(TriggerEvent(TerminalEvent(event)));
            }
        });
}
