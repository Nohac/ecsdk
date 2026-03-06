mod plain;
mod tui;

pub use tui::TerminalGuard;

use std::future::Future;
use std::pin::Pin;

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use clap::ValueEnum;
use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;

use crate::container::build_merged_log_view;
use crate::task::CommandSender;

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

type EventHandler =
    Box<dyn FnMut(Event, CommandSender) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

pub struct CrosstermPlugin {
    mode: RenderMode,
    on_event: Option<EventHandler>,
}

impl CrosstermPlugin {
    pub fn new(mode: RenderMode) -> Self {
        Self {
            mode,
            on_event: None,
        }
    }

    /// Register an async handler for terminal events.
    pub fn on_event<F, Fut>(mut self, mut handler: F) -> Self
    where
        F: FnMut(Event, CommandSender) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.on_event = Some(Box::new(move |event, cmd| Box::pin(handler(event, cmd))));
        self
    }

    /// Register ECS systems and return an optional async task for event polling.
    pub fn build(mut self, app: &mut App) -> Option<Pin<Box<dyn Future<Output = ()> + Send>>> {
        app.add_systems(Update, build_merged_log_view);
        match self.mode {
            RenderMode::Tui => {
                app.world_mut().insert_resource(TerminalGuard::new());
                app.insert_resource(TerminalSize::query_now());
                app.add_systems(Update, tui::render_tui.after(build_merged_log_view));

                let cmd_sender = app.world().resource::<CommandSender>().clone();
                let mut on_event = self.on_event.take();
                Some(Box::pin(async move {
                    let mut events = EventStream::new();
                    while let Some(Ok(event)) = events.next().await {
                        if let Event::Resize(cols, rows) = event {
                            cmd_sender.send(move |world: &mut World| {
                                world.resource_mut::<TerminalSize>().update(cols, rows);
                            });
                        }
                        if let Some(handler) = &mut on_event {
                            handler(event, cmd_sender.clone()).await;
                        }
                    }
                }))
            }
            RenderMode::Plain => {
                app.add_systems(Update, plain::render_plain);
                None
            }
        }
    }
}
