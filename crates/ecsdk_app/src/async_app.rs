use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Duration;

use bevy::app::App;
use bevy::ecs::prelude::World;
use ecsdk_core::{
    AppExit, ApplyMessage, CmdQueue, MessageQueue, QueueCmdExt, SendMsgExt, TickSignal,
    WakeSignal,
    WorldCallback,
};
use tokio::runtime::Handle;
use tokio::sync::{Notify, mpsc};
use tokio::time::interval;

pub struct Receivers<M: ApplyMessage> {
    pub(crate) state_rx: mpsc::UnboundedReceiver<M>,
    pub(crate) cmd_rx: mpsc::UnboundedReceiver<WorldCallback>,
    pub(crate) tick: Arc<Notify>,
    pub(crate) wake: Arc<Notify>,
}

/// An async-capable Bevy app bundled with the runtime receivers created by
/// [`setup`].
///
/// `AsyncApp` dereferences to `App`, so setup code can treat it like a normal
/// Bevy app and then call [`AsyncApp::run`] when ready.
pub struct AsyncApp<M: ApplyMessage> {
    app: App,
    receivers: Receivers<M>,
}

/// Creates a new [`AsyncApp`] and installs the core queue and signal resources
/// required by the `ecsdk` runtime loop.
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
    /// Splits the wrapper into its raw `App` and runtime receivers.
    ///
    /// Most applications should prefer [`AsyncApp::run`] instead.
    pub fn into_parts(self) -> (App, Receivers<M>) {
        (self.app, self.receivers)
    }

    /// Runs the app using the standard `ecsdk` async runtime loop.
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
    /// Enqueues a typed domain message from app-level setup code.
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

pub trait AppQueueCmdExt {
    /// Queues a direct world callback from app-level setup code.
    fn queue_cmd(&mut self, f: impl FnOnce(&mut World) + Send + 'static);
}

impl AppQueueCmdExt for App {
    fn queue_cmd(&mut self, f: impl FnOnce(&mut World) + Send + 'static) {
        self.world_mut().queue_cmd(f);
    }
}

impl<M: ApplyMessage> AppQueueCmdExt for AsyncApp<M> {
    fn queue_cmd(&mut self, f: impl FnOnce(&mut World) + Send + 'static) {
        self.app.queue_cmd(f);
    }
}

/// Runs the standard `ecsdk` async loop for an existing Bevy app and the
/// receiver set returned by [`setup`].
///
/// The loop drains typed state messages, queued world callbacks, immediate wake
/// notifications, and FPS-bounded tick notifications.
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
