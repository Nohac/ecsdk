use bevy::ecs::prelude::*;
use ecsdk_core::{ApplyMessage, CmdQueue, MessageQueue};
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

/// Handle passed to async task closures. Provides access to the owning entity,
/// a way to send world-mutating commands, and state events back to the ECS.
#[derive(Clone)]
pub struct TaskQueue<M: ApplyMessage> {
    entity: Entity,
    queue: CmdQueue,
    state_queue: MessageQueue<M>,
}

impl<M: ApplyMessage> TaskQueue<M> {
    pub fn new(entity: Entity, queue: CmdQueue, state_queue: MessageQueue<M>) -> Self {
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

    pub fn send_state(&self, event: M) {
        self.state_queue.send(event);
    }

    pub fn wake(&self) {
        self.queue.wake();
    }
}

/// A task handle without a message channel. Can only send world callbacks.
#[derive(Clone)]
pub struct CmdOnly {
    entity: Entity,
    queue: CmdQueue,
}

impl CmdOnly {
    pub(crate) fn new(entity: Entity, queue: CmdQueue) -> Self {
        Self { entity, queue }
    }

    pub fn entity(&self) -> Entity {
        self.entity
    }

    pub fn send(&self, f: impl FnOnce(&mut World) + Send + 'static) -> &Self {
        self.queue.send(f);
        self
    }

    pub fn wake(&self) {
        self.queue.wake();
    }
}
