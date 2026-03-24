use bevy::ecs::prelude::*;
use ecsdk_core::{ApplyMessage, CmdQueue, SendMsgExt};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Component tracking an in-flight async task. Cancels the task on drop.
#[derive(Component)]
pub struct AsyncTask {
    pub(crate) token: CancellationToken,
    pub(crate) _handle: JoinHandle<()>,
}

impl Drop for AsyncTask {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

#[derive(Event)]
pub struct TaskComplete(pub Entity);

#[derive(Event)]
pub struct TaskAborted(pub Entity);

/// Handle passed to async task closures.
///
/// `TaskQueue` carries the owning entity identity plus the two standard async
/// communication styles used throughout `ecsdk`:
///
/// - [`TaskQueue::send_msg`] for typed domain messages
/// - [`TaskQueue::queue_cmd_wake`] / [`TaskQueue::queue_cmd_tick`] for direct
///   world callbacks with explicit scheduling
///
/// This keeps task code aligned with the same `send_msg` /
/// `queue_cmd_wake` / `queue_cmd_tick` naming used by world, commands, and
/// app-level helpers.
#[derive(Clone)]
pub struct TaskQueue {
    entity: Entity,
    queue: CmdQueue,
}

impl TaskQueue {
    /// Creates a new task handle for the given owning entity.
    pub fn new(entity: Entity, queue: CmdQueue) -> Self {
        Self { entity, queue }
    }

    /// Returns the entity that owns this task.
    pub fn entity(&self) -> Entity {
        self.entity
    }

    /// Enqueues a direct world-mutating callback from async task code and
    /// requests an immediate update.
    pub fn queue_cmd_wake(&self, f: impl FnOnce(&mut World) + Send + 'static) -> &Self {
        let _ = self.queue.tx.send(Box::new(f));
        self.queue.wake();
        self
    }

    /// Enqueues a direct world-mutating callback from async task code and
    /// requests processing on the next tick-bound update.
    pub fn queue_cmd_tick(&self, f: impl FnOnce(&mut World) + Send + 'static) -> &Self {
        let _ = self.queue.tx.send(Box::new(f));
        self.queue.tick();
        self
    }

    /// Enqueues a typed domain message from async task code.
    pub fn send_msg<M: ApplyMessage>(&self, msg: M) {
        let _ = self.queue.tx.send(Box::new(move |world: &mut World| {
            world.send_msg(msg);
        }));
    }

    /// Requests an immediate schedule run after queued work has been submitted.
    pub fn wake(&self) {
        self.queue.wake();
    }
}
