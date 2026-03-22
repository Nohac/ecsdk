mod plain;
mod tui;

pub use ecsdk::term::TerminalGuard;

use ecsdk::prelude::*;
use clap::ValueEnum;
use ecsdk::term::TermPlugin;

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum RenderMode {
    Plain,
    Tui,
    None,
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
        match self.mode {
            RenderMode::Tui => {
                app.insert_resource(TerminalGuard::new());
                app.add_plugins(TermPlugin);
                app.add_systems(PostUpdate, tui::render_tui);
            }
            RenderMode::Plain => {
                app.add_systems(PostUpdate, plain::render_plain);
            }
            RenderMode::None => {}
        }
    }
}
