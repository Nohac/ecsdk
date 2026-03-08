use std::sync::Arc;
use std::time::Duration;

use bevy::app::App;
use bevy::ecs::prelude::World;
use tokio::runtime::Handle;
use tokio::sync::{Notify, mpsc};
use tokio::time::interval;

use crate::msg::{AppExit, CmdQueue, TickSignal, WakeSignal, WorldCallback};
use crate::message::{Message, MessageQueue};

pub struct Receivers {
    state_rx: mpsc::UnboundedReceiver<Message>,
    cmd_rx: mpsc::UnboundedReceiver<WorldCallback>,
    tick: Arc<Notify>,
    wake: Arc<Notify>,
}

pub fn setup() -> (App, Receivers) {
    let (state_tx, state_rx) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let tick = Arc::new(Notify::new());
    let wake = Arc::new(Notify::new());

    let mut app = App::new();
    app.insert_resource(CmdQueue::new(
        cmd_tx,
        Handle::current(),
        WakeSignal(wake.clone()),
    ));
    app.insert_resource(MessageQueue::new(state_tx));
    app.insert_resource(TickSignal(tick.clone()));
    app.insert_resource(WakeSignal(wake.clone()));
    app.init_resource::<AppExit>();

    (
        app,
        Receivers {
            state_rx,
            cmd_rx,
            tick,
            wake,
        },
    )
}

pub async fn run_async(mut app: App, mut rx: Receivers) {
    let mut tick_interval = interval(Duration::from_millis(1000 / 5));
    let mut needs_tick = false;

    app.finish();
    app.cleanup();
    app.update();

    loop {
        tokio::select! {
            biased;

            Some(event) = rx.state_rx.recv() => {
                event.apply(app.world_mut());
                while let Ok(event) = rx.state_rx.try_recv() {
                    event.apply(app.world_mut());
                }
                drain_cmds(app.world_mut(), &mut rx.cmd_rx);
                app.update();
            }

            Some(cb) = rx.cmd_rx.recv() => {
                cb(app.world_mut());
                drain_cmds(app.world_mut(), &mut rx.cmd_rx);
            }

            _ = rx.wake.notified() => {
                drain_cmds(app.world_mut(), &mut rx.cmd_rx);
                app.update();
            }

            _ = rx.tick.notified() => {
                needs_tick = true;
            }

            _ = tick_interval.tick(), if needs_tick => {
                drain_cmds(app.world_mut(), &mut rx.cmd_rx);
                app.update();
                needs_tick = false;
            }
        }

        if app.world().resource::<AppExit>().0 {
            break;
        }
    }
}

fn drain_cmds(world: &mut World, rx: &mut mpsc::UnboundedReceiver<WorldCallback>) {
    while let Ok(cb) = rx.try_recv() {
        cb(world);
    }
}
