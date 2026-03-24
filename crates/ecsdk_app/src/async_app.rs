use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Duration;

use bevy::app::App;
use bevy::ecs::prelude::World;
use ecsdk_core::{
    ApplyMessage, CmdQueue, MessageQueue, QueueCmdExt, SendMsgExt, TickSignal, WakeSignal,
    WorldCallback,
};
use tokio::runtime::Handle;
use tokio::sync::{Notify, mpsc};
use tokio::time::interval;

pub struct Receivers<M: ApplyMessage> {
    pub(crate) msg_rx: mpsc::UnboundedReceiver<M>,
    pub(crate) cmd_rx: mpsc::UnboundedReceiver<WorldCallback>,
    pub(crate) tick: Arc<Notify>,
    pub(crate) wake: Arc<Notify>,
}

/// Runtime tuning for the `ecsdk` async loop.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeConfig {
    tick_rate_hz: u32,
}

impl RuntimeConfig {
    /// Creates a runtime configuration with the default tick cadence.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the configured tick rate in hertz.
    pub fn tick_rate_hz(&self) -> u32 {
        self.tick_rate_hz
    }

    /// Sets the tick rate used for tick-bound updates.
    ///
    /// Panics if `tick_rate_hz` is zero.
    pub fn with_tick_rate_hz(mut self, tick_rate_hz: u32) -> Self {
        assert!(tick_rate_hz > 0, "tick_rate_hz must be greater than zero");
        self.tick_rate_hz = tick_rate_hz;
        self
    }

    fn tick_interval(self) -> Duration {
        Duration::from_secs_f64(1.0 / self.tick_rate_hz as f64)
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self { tick_rate_hz: 5 }
    }
}

/// An async-capable Bevy app bundled with the runtime receivers created by
/// [`setup`].
///
/// `AsyncApp` dereferences to `App`, so setup code can treat it like a normal
/// Bevy app and then call [`AsyncApp::run`] when ready.
pub struct AsyncApp<M: ApplyMessage> {
    app: App,
    receivers: Receivers<M>,
    runtime: RuntimeConfig,
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
        TickSignal(tick.clone()),
        WakeSignal(wake.clone()),
    ));
    app.insert_resource(MessageQueue::<M>::new(state_tx));
    app.insert_resource(TickSignal(tick.clone()));
    app.insert_resource(WakeSignal(wake.clone()));

    AsyncApp {
        app,
        receivers: Receivers {
            msg_rx: state_rx,
            cmd_rx,
            tick,
            wake,
        },
        runtime: RuntimeConfig::default(),
    }
}

impl<M: ApplyMessage> AsyncApp<M> {
    /// Splits the wrapper into its raw `App` and runtime receivers.
    ///
    /// Most applications should prefer [`AsyncApp::run`] instead.
    pub fn into_parts(self) -> (App, Receivers<M>) {
        (self.app, self.receivers)
    }

    /// Returns the current runtime configuration.
    pub fn runtime_config(&self) -> RuntimeConfig {
        self.runtime
    }

    /// Replaces the runtime configuration used when [`AsyncApp::run`] is
    /// called.
    pub fn set_runtime_config(&mut self, runtime: RuntimeConfig) -> &mut Self {
        self.runtime = runtime;
        self
    }

    /// Sets the tick cadence used for tick-bound updates.
    ///
    /// Panics if `tick_rate_hz` is zero.
    pub fn set_tick_rate_hz(&mut self, tick_rate_hz: u32) -> &mut Self {
        self.runtime = self.runtime.with_tick_rate_hz(tick_rate_hz);
        self
    }

    /// Runs the app using the standard `ecsdk` async runtime loop.
    pub async fn run(mut self) {
        run_async(&mut self.app, self.receivers, self.runtime).await;
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
    /// Queues a direct world callback from app-level setup code and requests an
    /// immediate update.
    fn queue_cmd_wake(&mut self, f: impl FnOnce(&mut World) + Send + 'static);

    /// Queues a direct world callback from app-level setup code and requests a
    /// tick-bound update.
    fn queue_cmd_tick(&mut self, f: impl FnOnce(&mut World) + Send + 'static);
}

impl AppQueueCmdExt for App {
    fn queue_cmd_wake(&mut self, f: impl FnOnce(&mut World) + Send + 'static) {
        self.world_mut().queue_cmd_wake(f);
    }

    fn queue_cmd_tick(&mut self, f: impl FnOnce(&mut World) + Send + 'static) {
        self.world_mut().queue_cmd_tick(f);
    }
}

impl<M: ApplyMessage> AppQueueCmdExt for AsyncApp<M> {
    fn queue_cmd_wake(&mut self, f: impl FnOnce(&mut World) + Send + 'static) {
        self.app.queue_cmd_wake(f);
    }

    fn queue_cmd_tick(&mut self, f: impl FnOnce(&mut World) + Send + 'static) {
        self.app.queue_cmd_tick(f);
    }
}

/// Runs the standard `ecsdk` async loop for an existing Bevy app and the
/// receiver set returned by [`setup`].
///
/// The loop drains typed state messages, queued world callbacks, immediate wake
/// notifications, and FPS-bounded tick notifications.
pub async fn run_async<M: ApplyMessage>(
    app: &mut App,
    mut rx: Receivers<M>,
    runtime: RuntimeConfig,
) {
    let mut tick_interval = interval(runtime.tick_interval());
    let mut needs_tick = false;

    app.finish();
    app.cleanup();
    app.update();
    if app.should_exit().is_some() {
        return;
    }

    loop {
        tokio::select! {
            biased;

            Some(event) = rx.msg_rx.recv() => {
                event.apply(app.world_mut());
                while let Ok(event) = rx.msg_rx.try_recv() {
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

        if app.should_exit().is_some() {
            break;
        }
    }
}

fn drain_cmds(world: &mut World, rx: &mut mpsc::UnboundedReceiver<WorldCallback>) {
    while let Ok(cb) = rx.try_recv() {
        cb(world);
    }
}
