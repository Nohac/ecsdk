use std::sync::Arc;

use bevy::ecs::prelude::*;
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::sync::mpsc;

use crate::message::{Message, MessageQueue};

/// Boxed closure that mutates the world directly from an async task.
pub type WorldCallback = Box<dyn FnOnce(&mut World) + Send>;

/// Signals the main loop to run `app.update()` at the next fps boundary.
#[derive(Resource, Clone)]
pub struct TickSignal(pub(crate) Arc<Notify>);

/// Signals the main loop to run `app.update()` immediately.
#[derive(Resource, Clone)]
pub struct WakeSignal(pub(crate) Arc<Notify>);

/// Resource that bridges async tasks to the ECS world.
/// Carries a command channel sender and an optional Tokio runtime handle.
#[derive(Resource, Clone)]
pub struct CmdQueue {
    pub(crate) tx: mpsc::UnboundedSender<WorldCallback>,
    pub(crate) handle: Option<Handle>,
    pub(crate) wake: WakeSignal,
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

/// Handle passed to async task closures. Provides access to the owning entity,
/// a way to send world-mutating commands, and state events back to the ECS.
#[derive(Clone)]
pub struct TaskQueue {
    entity: Entity,
    queue: CmdQueue,
    state_queue: MessageQueue,
}

impl TaskQueue {
    pub(crate) fn new(entity: Entity, queue: CmdQueue, state_queue: MessageQueue) -> Self {
        Self {
            entity,
            queue,
            state_queue,
        }
    }

    pub fn entity(&self) -> Entity {
        self.entity
    }

    pub fn send(&self, f: impl FnOnce(&mut World) + Send + 'static) -> &Self {
        self.queue.send(f);
        self
    }

    pub fn send_state(&self, event: Message) {
        self.state_queue.send(event);
    }

    pub fn wake(&self) {
        self.queue.wake();
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
