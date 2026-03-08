use std::sync::Arc;
use std::time::Duration;

use bevy::app::App;
use bevy::ecs::prelude::World;
use tokio::runtime::Handle;
use tokio::sync::{Notify, mpsc};
use tokio::time::interval;

use crate::msg::{AppExit, NeedsTick, NeedsWake, Queue, WorldCallback};
use crate::state_event::{StateEvent, StateQueue};

pub struct Receivers {
    state_rx: mpsc::UnboundedReceiver<StateEvent>,
    cmd_rx: mpsc::UnboundedReceiver<WorldCallback>,
    wake: Arc<Notify>,
}

pub fn setup() -> (App, Receivers) {
    let (state_tx, state_rx) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let wake = Arc::new(Notify::new());

    let mut app = App::new();
    app.insert_resource(Queue::new(cmd_tx, Handle::current(), wake.clone()));
    app.insert_resource(StateQueue::new(state_tx));
    app.init_resource::<AppExit>();
    app.init_resource::<NeedsTick>();
    app.init_resource::<NeedsWake>();

    (
        app,
        Receivers {
            state_rx,
            cmd_rx,
            wake,
        },
    )
}

pub async fn run_async(mut app: App, mut rx: Receivers) {
    let mut tick_interval = interval(Duration::from_millis(1000 / 30)); // ~15fps
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
                app.world_mut().flush();
            }

            _ = rx.wake.notified() => {
                drain_cmds(app.world_mut(), &mut rx.cmd_rx);
                app.update();
            }

            _ = tick_interval.tick(), if needs_tick => {
                app.update();
                needs_tick = false;
            }
        }

        if app.world().resource::<NeedsWake>().0 {
            app.world_mut().resource_mut::<NeedsWake>().0 = false;
            needs_tick = false;
            continue;
        }
        if app.world().resource::<NeedsTick>().0 {
            app.world_mut().resource_mut::<NeedsTick>().0 = false;
            needs_tick = true;
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
