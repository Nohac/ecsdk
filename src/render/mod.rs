mod plain;
mod tui;

pub use tui::TerminalGuard;

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use clap::ValueEnum;
use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;

use crate::app::TaskQueue;
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
    on_event: Mutex<Option<EventHandler>>,
}

impl CrosstermPlugin {
    pub fn new(mode: RenderMode) -> Self {
        Self {
            mode,
            on_event: Mutex::new(None),
        }
    }

    /// Register an async handler for terminal events. Called for every crossterm
    /// event; resize is handled automatically before the callback is invoked.
    ///
    /// The handler must return an owned future (`async move { ... }`), so
    /// captured state needs cloning before the async block:
    /// ```ignore
    /// CrosstermPlugin::new(mode).on_event(move |event, _cmd| {
    ///     let c = client.clone();
    ///     async move { let _ = c.shutdown().await; }
    /// })
    /// ```
    pub fn on_event<F, Fut>(self, mut handler: F) -> Self
    where
        F: FnMut(Event, CommandSender) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        *self.on_event.lock().unwrap() =
            Some(Box::new(move |event, cmd| Box::pin(handler(event, cmd))));
        self
    }
}

impl Plugin for CrosstermPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.add_systems(Update, build_merged_log_view);
        match self.mode {
            RenderMode::Tui => {
                app.world_mut().insert_resource(TerminalGuard::new());
                app.insert_resource(TerminalSize::query_now());
                app.add_systems(Update, tui::render_tui.after(build_merged_log_view));

                let cmd_sender = app.world().resource::<CommandSender>().clone();
                let mut on_event = self.on_event.lock().unwrap().take();
                app.world_mut()
                    .resource_mut::<TaskQueue>()
                    .push(async move {
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
                    });
            }
            RenderMode::Plain => {
                app.add_systems(Update, plain::render_plain);
            }
        }
    }
}
