use std::sync::Arc;
use std::time::Duration;
use std::ops::{Deref, DerefMut};

use bevy::app::App;
use bevy::ecs::prelude::World;
use ecsdk_core::{
    AppExit, ApplyMessage, CmdQueue, MessageQueue, SendMsgExt, TickSignal, WakeSignal,
    WorldCallback,
};
use tokio::runtime::Handle;
use tokio::sync::{Notify, mpsc};
use tokio::time::interval;

pub struct Receivers<M: ApplyMessage> {
    state_rx: mpsc::UnboundedReceiver<M>,
    cmd_rx: mpsc::UnboundedReceiver<WorldCallback>,
    tick: Arc<Notify>,
    wake: Arc<Notify>,
}

pub struct AsyncApp<M: ApplyMessage> {
    app: App,
    receivers: Receivers<M>,
}

pub fn setup<M: ApplyMessage>() -> AsyncApp<M> {
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
    app.insert_resource(MessageQueue::<M>::new(state_tx));
    app.insert_resource(TickSignal(tick.clone()));
    app.insert_resource(WakeSignal(wake.clone()));
    app.init_resource::<AppExit>();

    AsyncApp {
        app,
        receivers: Receivers {
            state_rx,
            cmd_rx,
            tick,
            wake,
        },
    }
}

impl<M: ApplyMessage> AsyncApp<M> {
    pub fn into_parts(self) -> (App, Receivers<M>) {
        (self.app, self.receivers)
    }

    pub async fn run(mut self) {
        run_async(&mut self.app, self.receivers).await;
    }
}

impl<M: ApplyMessage> Deref for AsyncApp<M> {
    type Target = App;

    fn deref(&self) -> &Self::Target {
        &self.app
    }
}

impl<M: ApplyMessage> DerefMut for AsyncApp<M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.app
    }
}

pub trait AppSendMsgExt {
    fn send_msg<M: ApplyMessage>(&mut self, msg: M);
}

impl AppSendMsgExt for App {
    fn send_msg<M: ApplyMessage>(&mut self, msg: M) {
        self.world_mut().send_msg(msg);
    }
}

impl<M: ApplyMessage> AppSendMsgExt for AsyncApp<M> {
    fn send_msg<T: ApplyMessage>(&mut self, msg: T) {
        self.app.send_msg(msg);
    }
}

pub async fn run_async<M: ApplyMessage>(app: &mut App, mut rx: Receivers<M>) {
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
