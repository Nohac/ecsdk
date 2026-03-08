use std::sync::Arc;

use bevy::ecs::prelude::*;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::sync::Notify;

use crate::state_event::{StateEvent, StateQueue};

/// Boxed closure that mutates the world directly from an async task.
pub type WorldCallback = Box<dyn FnOnce(&mut World) + Send>;

/// Resource that bridges async tasks to the ECS world.
/// Carries a command channel sender, an optional Tokio runtime handle,
/// and a wake signal for triggering immediate schedule updates.
#[derive(Resource, Clone)]
pub struct Queue {
    pub(crate) tx: mpsc::UnboundedSender<WorldCallback>,
    pub(crate) handle: Option<Handle>,
    pub(crate) wake: Arc<Notify>,
}

impl Queue {
    pub fn new(tx: mpsc::UnboundedSender<WorldCallback>, handle: Handle, wake: Arc<Notify>) -> Self {
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
            wake: Arc::new(Notify::new()),
        }
    }

    pub fn send(&self, f: impl FnOnce(&mut World) + Send + 'static) {
        let _ = self.tx.send(Box::new(f));
    }

    pub fn wake(&self) {
        self.wake.notify_one();
    }
}

/// Handle passed to async task closures. Provides access to the owning entity,
/// a way to send world-mutating commands, and state events back to the ECS.
#[derive(Clone)]
pub struct TaskQueue {
    entity: Entity,
    queue: Queue,
    state_queue: StateQueue,
}

impl TaskQueue {
    pub(crate) fn new(entity: Entity, queue: Queue, state_queue: StateQueue) -> Self {
        Self {
            entity,
            queue,
            state_queue,
        }
    }

    pub fn entity(&self) -> Entity {
        self.entity
    }

    pub fn send(&self, f: impl FnOnce(&mut World) + Send + 'static) {
        self.queue.send(f);
    }

    pub fn send_state(&self, event: StateEvent) {
        self.state_queue.send(event);
    }

    pub fn wake(&self) {
        self.queue.wake();
    }
}

// ── Scheduling resources ──

/// Set by systems/observers to request a tick-rate-limited schedule update.
#[derive(Resource, Default)]
pub struct NeedsTick(pub bool);

/// Set by systems/observers to request an immediate schedule update.
#[derive(Resource, Default)]
pub struct NeedsWake(pub bool);

/// Extension trait for requesting schedule updates from within `app.update()`.
pub trait ScheduleControl {
    fn tick(&mut self);
    fn wake(&mut self);
}

impl ScheduleControl for Commands<'_, '_> {
    fn tick(&mut self) {
        self.insert_resource(NeedsTick(true));
    }
    fn wake(&mut self) {
        self.insert_resource(NeedsWake(true));
    }
}

/// Checked by main loop after each cycle to know when to exit.
#[derive(Resource, Default)]
pub struct AppExit(pub bool);
