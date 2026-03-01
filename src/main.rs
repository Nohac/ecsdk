use std::io::IsTerminal;

use bevy_ecs::prelude::*;
use clap::Parser;
use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use tokio::{select, signal::ctrl_c, sync::mpsc};

use ecstest::bridge::{AppExit, EventSender, TokioHandle, WorldCmd};
use ecstest::container::build_startup_schedule;
use ecstest::lifecycle::{ShutdownAll, build_update_schedule, register_observers};
use ecstest::render::{
    RenderMode, TerminalSize, build_render_schedule, install_panic_hook, terminal_init,
    terminal_teardown,
};

#[derive(Parser)]
#[command(about = "ECS-driven container orchestration demo")]
struct Cli {
    /// Output mode (plain or tui). Defaults to tui when stdout is a terminal.
    #[arg(long, value_enum)]
    output: Option<RenderMode>,
}

fn resolve_render_mode(explicit: Option<RenderMode>) -> RenderMode {
    explicit.unwrap_or_else(|| {
        if std::io::stdout().is_terminal() {
            RenderMode::Tui
        } else {
            RenderMode::Plain
        }
    })
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let mode = resolve_render_mode(cli.output);

    if mode == RenderMode::Tui {
        install_panic_hook();
        terminal_init();
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<WorldCmd>();
    let tx_ctrl_c = EventSender(tx.clone());
    let tx_term = EventSender(tx.clone());

    let mut world = World::new();
    world.insert_resource(EventSender(tx));
    world.insert_resource(TokioHandle(tokio::runtime::Handle::current()));
    world.insert_resource(TerminalSize::query_now());
    world.init_resource::<AppExit>();

    register_observers(&mut world);

    let mut startup = build_startup_schedule();
    let mut update = build_update_schedule();
    let mut render = build_render_schedule(mode);

    startup.run(&mut world);
    update.run(&mut world);
    render.run(&mut world);

    let mut term_events = EventStream::new();

    loop {
        select! {
            Some(cmd) = rx.recv() => {
                cmd(&mut world);
                update.run(&mut world);
                render.run(&mut world);
                if world.resource::<AppExit>().0 { break; }
            }
            Some(Ok(event)) = term_events.next(), if mode == RenderMode::Tui => {
                if let Event::Resize(cols, rows) = event {
                    tx_term.send(move |world: &mut World| {
                        world.resource_mut::<TerminalSize>().update(cols, rows);
                    });
                }
            }
            _ = ctrl_c() => {
                tx_ctrl_c.trigger(ShutdownAll);
            }
        }
    }

    if mode == RenderMode::Tui {
        terminal_teardown();
    }
}
