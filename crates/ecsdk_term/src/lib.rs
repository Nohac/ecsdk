use std::io::{Write, stdout};

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use crossterm::event::{Event, EventStream};
use crossterm::{
    ExecutableCommand,
    cursor::{Hide, Show},
    terminal::{
        DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen,
        disable_raw_mode, enable_raw_mode,
    },
};
use ecsdk_core::ScheduleControl;
use ecsdk_tasks::SpawnCmdTask;
use futures_util::StreamExt;

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

/// A screen rectangle: col, row, width, height (all in terminal cells).
#[derive(Clone, Copy)]
pub struct Rect {
    pub col: u16,
    pub row: u16,
    pub w: u16,
    pub h: u16,
}

/// Set the terminal scroll region via raw ANSI escape (CSI r).
/// top and bottom are 1-based row numbers.
pub fn set_scroll_region(out: &mut impl Write, top: u16, bottom: u16) {
    let _ = write!(out, "\x1b[{};{}r", top, bottom);
}

/// Reset scroll region to full terminal.
pub fn reset_scroll_region(out: &mut impl Write) {
    let _ = write!(out, "\x1b[r");
}

/// RAII guard that initializes the terminal on creation and restores it on drop.
#[derive(Resource, Default)]
pub struct TerminalGuard;

impl TerminalGuard {
    pub fn new() -> Self {
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            terminal_teardown();
            original(info);
        }));
        let mut out = stdout();
        let _ = enable_raw_mode();
        let _ = out.execute(EnterAlternateScreen);
        let _ = out.execute(Hide);
        let _ = out.execute(DisableLineWrap);
        Self
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        terminal_teardown();
    }
}

fn terminal_teardown() {
    let mut out = stdout();
    reset_scroll_region(&mut out);
    let _ = out.execute(EnableLineWrap);
    let _ = out.execute(Show);
    let _ = out.execute(LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

/// Plugin that sets up terminal size tracking and the crossterm event reader task.
pub struct TermPlugin;

impl Plugin for TermPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TerminalSize::query_now());
        app.add_systems(Startup, setup_crossterm);
    }
}

fn setup_crossterm(mut commands: Commands) {
    commands
        .spawn(CrosstermEntity)
        .spawn_cmd_task(|cmd| async move {
            let mut events = EventStream::new();
            while let Some(Ok(event)) = events.next().await {
                if let Event::Resize(cols, rows) = event {
                    cmd.send(move |world: &mut World| {
                        let mut size = world.resource_mut::<TerminalSize>();
                        size.cols = cols;
                        size.rows = rows;
                        world.tick();
                    });
                }
                let event_clone = event.clone();
                cmd.send(move |world: &mut World| {
                    world.trigger(TerminalEvent(event_clone));
                })
                .wake();
            }
        });
}
