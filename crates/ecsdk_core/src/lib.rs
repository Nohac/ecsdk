use std::sync::Arc;

use bevy::ecs::prelude::*;
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::sync::mpsc;

/// Boxed closure that mutates the world directly from an async task.
pub type WorldCallback = Box<dyn FnOnce(&mut World) + Send>;

/// Signals the main loop to run `app.update()` at the next fps boundary.
#[derive(Resource, Clone)]
pub struct TickSignal(pub Arc<Notify>);

/// Signals the main loop to run `app.update()` immediately.
#[derive(Resource, Clone)]
pub struct WakeSignal(pub Arc<Notify>);

/// Resource that bridges async tasks to the ECS world.
/// Carries a command channel sender and an optional Tokio runtime handle.
#[derive(Resource, Clone)]
pub struct CmdQueue {
    pub tx: mpsc::UnboundedSender<WorldCallback>,
    pub handle: Option<Handle>,
    pub wake: WakeSignal,
}

impl CmdQueue {
    pub fn new(tx: mpsc::UnboundedSender<WorldCallback>, handle: Handle, wake: WakeSignal) -> Self {
        Self {
            tx,
            handle: Some(handle),
            wake,
        }
    }

    /// Creates a no-op queue for testing. Commands are silently dropped
    /// and async task spawning is disabled (no Tokio runtime needed).
    pub fn test() -> Self {
        let (tx, _) = mpsc::unbounded_channel();
        Self {
            tx,
            handle: None,
            wake: WakeSignal(Arc::new(Notify::new())),
        }
    }

    pub fn send(&self, f: impl FnOnce(&mut World) + Send + 'static) -> &Self {
        let _ = self.tx.send(Box::new(f));
        self
    }

    pub fn wake(&self) {
        self.wake.0.notify_one();
    }
}

// ── Scheduling ──

/// Extension trait for requesting schedule updates.
pub trait ScheduleControl {
    fn tick(&mut self);
    fn wake(&mut self);
}

impl ScheduleControl for Commands<'_, '_> {
    fn tick(&mut self) {
        self.queue(|world: &mut World| world.tick());
    }
    fn wake(&mut self) {
        self.queue(|world: &mut World| world.wake());
    }
}

impl ScheduleControl for World {
    fn tick(&mut self) {
        self.resource::<TickSignal>().0.notify_one();
    }
    fn wake(&mut self) {
        self.resource::<WakeSignal>().0.notify_one();
    }
}

/// Checked by main loop after each cycle to know when to exit.
#[derive(Resource, Default)]
pub struct AppExit(pub bool);

// ── ApplyMessage trait ──

/// Trait for domain message types that can mutate the world.
pub trait ApplyMessage: Send + 'static {
    fn apply(&self, world: &mut World);
}

// ── Generic MessageQueue ──

#[derive(Resource)]
pub struct MessageQueue<M: ApplyMessage> {
    tx: mpsc::UnboundedSender<M>,
}

impl<M: ApplyMessage> Clone for MessageQueue<M> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<M: ApplyMessage> MessageQueue<M> {
    pub fn new(tx: mpsc::UnboundedSender<M>) -> Self {
        Self { tx }
    }

    pub fn send(&self, msg: M) {
        let _ = self.tx.send(msg);
    }

    pub fn test() -> Self {
        let (tx, _) = mpsc::unbounded_channel();
        Self { tx }
    }
}

pub trait SendMsgExt {
    fn send_msg<M: ApplyMessage>(&mut self, msg: M);
}

impl SendMsgExt for World {
    fn send_msg<M: ApplyMessage>(&mut self, msg: M) {
        self.resource::<MessageQueue<M>>().send(msg);
    }
}
