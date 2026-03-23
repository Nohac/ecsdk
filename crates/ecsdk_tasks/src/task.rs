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
/// - [`TaskQueue::queue_cmd`] for direct world callbacks
///
/// This keeps task code aligned with the same `send_msg` / `queue_cmd` naming
/// used by world, commands, and app-level helpers.
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

    /// Enqueues a direct world-mutating callback from async task code.
    pub fn queue_cmd(&self, f: impl FnOnce(&mut World) + Send + 'static) -> &Self {
        self.queue.queue_cmd(f);
        self
    }

    /// Enqueues a typed domain message from async task code.
    pub fn send_msg<M: ApplyMessage>(&self, msg: M) {
        self.queue.queue_cmd(move |world: &mut World| {
            world.send_msg(msg);
        });
    }

    /// Requests an immediate schedule run after queued work has been submitted.
    pub fn wake(&self) {
        self.queue.wake();
    }
}
