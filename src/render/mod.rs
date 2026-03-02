mod plain;
mod tui;

pub use tui::{install_panic_hook, terminal_init, terminal_teardown};

use bevy_ecs::prelude::*;
use clap::ValueEnum;

use crate::container::build_merged_log_view;

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

    pub fn update(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
    }
}

pub fn build_render_schedule(mode: RenderMode) -> Schedule {
    let mut schedule = Schedule::default();
    match mode {
        RenderMode::Plain => {
            schedule.add_systems(plain::render_plain);
        }
        RenderMode::Tui => {
            schedule.add_systems(build_merged_log_view.before(tui::render_tui));
            schedule.add_systems(tui::render_tui);
        }
    }
    schedule
}
