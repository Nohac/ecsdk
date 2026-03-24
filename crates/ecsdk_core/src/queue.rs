use std::sync::Arc;

use bevy::ecs::prelude::*;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::{ApplyMessage, ScheduleControl, TickSignal, WakeSignal};

/// Boxed closure that mutates the world directly from an async task.
pub type WorldCallback = Box<dyn FnOnce(&mut World) + Send>;

/// Resource that bridges async tasks to the ECS world.
/// Carries a command channel sender, schedule signals, and an optional Tokio
/// runtime handle.
#[derive(Resource, Clone)]
pub struct CmdQueue {
    pub tx: mpsc::UnboundedSender<WorldCallback>,
    pub handle: Option<Handle>,
    pub tick: TickSignal,
    pub wake: WakeSignal,
}

impl CmdQueue {
    pub fn new(
        tx: mpsc::UnboundedSender<WorldCallback>,
        handle: Handle,
        tick: TickSignal,
        wake: WakeSignal,
    ) -> Self {
        Self {
            tx,
            handle: Some(handle),
            tick,
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
            tick: TickSignal(Arc::new(tokio::sync::Notify::new())),
            wake: WakeSignal(Arc::new(tokio::sync::Notify::new())),
        }
    }

    /// Requests an immediate schedule run from the async runtime loop.
    pub fn wake(&self) {
        self.wake.0.notify_one();
    }

    /// Requests processing on the next tick-bound update.
    pub fn tick(&self) {
        self.tick.0.notify_one();
    }
}

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

    /// Sends a typed domain message to the async runtime loop.
    pub fn send(&self, msg: M) {
        let _ = self.tx.send(msg);
    }

    pub fn test() -> Self {
        let (tx, _) = mpsc::unbounded_channel();
        Self { tx }
    }
}

/// Convenience methods for sending typed domain messages from ECS-side APIs.
///
/// This is the message-oriented counterpart to [`QueueCmdExt`].
pub trait SendMsgExt {
    /// Enqueues a typed domain message for later application to the world.
    fn send_msg<M: ApplyMessage>(&mut self, msg: M);
}

impl SendMsgExt for World {
    fn send_msg<M: ApplyMessage>(&mut self, msg: M) {
        self.resource::<MessageQueue<M>>().send(msg);
    }
}

impl SendMsgExt for Commands<'_, '_> {
    fn send_msg<M: ApplyMessage>(&mut self, msg: M) {
        self.queue(move |world: &mut World| {
            world.send_msg(msg);
        });
    }
}

/// Convenience methods for queueing direct world callbacks from ECS-side APIs
/// with explicit scheduling behavior.
///
/// This is the callback-oriented counterpart to [`SendMsgExt`]. Unlike typed
/// messages, direct callbacks do not carry their own scheduling semantics, so
/// callers must decide whether the follow-up work should run immediately or on
/// the next tick-bound update.
pub trait QueueCmdExt {
    /// Queues a callback and requests an immediate `app.update()`.
    ///
    /// Use this when the queued world mutation should be observed by systems as
    /// soon as possible.
    fn queue_cmd_wake(&mut self, f: impl FnOnce(&mut World) + Send + 'static);

    /// Queues a callback and requests processing on the next tick-bound update.
    ///
    /// Use this when the queued world mutation is render-oriented or can wait
    /// for the next bounded frame.
    fn queue_cmd_tick(&mut self, f: impl FnOnce(&mut World) + Send + 'static);
}

impl QueueCmdExt for World {
    fn queue_cmd_wake(&mut self, f: impl FnOnce(&mut World) + Send + 'static) {
        f(self);
        self.wake();
    }

    fn queue_cmd_tick(&mut self, f: impl FnOnce(&mut World) + Send + 'static) {
        f(self);
        self.tick();
    }
}

impl QueueCmdExt for Commands<'_, '_> {
    fn queue_cmd_wake(&mut self, f: impl FnOnce(&mut World) + Send + 'static) {
        self.queue(move |world: &mut World| {
            f(world);
            world.wake();
        });
    }

    fn queue_cmd_tick(&mut self, f: impl FnOnce(&mut World) + Send + 'static) {
        self.queue(move |world: &mut World| {
            f(world);
            world.tick();
        });
    }
}
