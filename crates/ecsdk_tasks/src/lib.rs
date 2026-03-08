use std::future::Future;

use bevy::ecs::prelude::*;
use ecsdk_core::{ApplyMessage, CmdQueue, MessageQueue};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Component tracking an in-flight async task. Cancels the task on drop.
#[derive(Component)]
pub struct AsyncTask {
    token: CancellationToken,
    _handle: JoinHandle<()>,
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

// ── TaskQueue (generic over message type) ──

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

// ── SpawnTask (generic over message type) ──

/// Extension trait for spawning entity-owned async tasks with message support.
pub trait SpawnTask<M: ApplyMessage> {
    fn spawn_task<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(TaskQueue<M>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static;
}

impl<M: ApplyMessage> SpawnTask<M> for EntityCommands<'_> {
    fn spawn_task<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(TaskQueue<M>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.queue(SpawnTaskCmd::<F, M>(f, std::marker::PhantomData));
        self
    }
}

struct SpawnTaskCmd<F, M>(F, std::marker::PhantomData<M>);

impl<F, Fut, M> EntityCommand for SpawnTaskCmd<F, M>
where
    F: FnOnce(TaskQueue<M>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
    M: ApplyMessage,
{
    fn apply(self, ewm: EntityWorldMut<'_>) {
        let entity = ewm.id();
        let world = ewm.into_world_mut();

        let queue = world.resource::<CmdQueue>().clone();
        let Some(rt_handle) = queue.handle.clone() else {
            return;
        };
        let state_queue = world.resource::<MessageQueue<M>>().clone();
        let token = CancellationToken::new();
        let task_queue = TaskQueue::new(entity, queue, state_queue);
        let fut = (self.0)(task_queue);
        let child_token = token.child_token();

        let entity_for_complete = entity;
        let entity_for_abort = entity;

        let handle = {
            let queue_complete = world.resource::<CmdQueue>().clone();
            let queue_abort = world.resource::<CmdQueue>().clone();
            rt_handle.spawn(async move {
                tokio::select! {
                    _ = fut => {
                        queue_complete
                            .send(move |world: &mut World| {
                                world.trigger(TaskComplete(entity_for_complete));
                            })
                            .wake();
                    }
                    _ = child_token.cancelled() => {
                        queue_abort
                            .send(move |world: &mut World| {
                                world.trigger(TaskAborted(entity_for_abort));
                            })
                            .wake();
                    }
                }
            })
        };

        world.entity_mut(entity).insert(AsyncTask {
            token,
            _handle: handle,
        });
    }
}

// ── CmdOnly (non-generic task handle) ──

/// A task handle without a message channel. Can only send world callbacks.
#[derive(Clone)]
pub struct CmdOnly {
    entity: Entity,
    queue: CmdQueue,
}

impl CmdOnly {
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

// ── SpawnCmdTask (non-generic) ──

/// Extension trait for spawning entity-owned async tasks that only need world callbacks.
pub trait SpawnCmdTask {
    fn spawn_cmd_task<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(CmdOnly) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static;
}

impl SpawnCmdTask for EntityCommands<'_> {
    fn spawn_cmd_task<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(CmdOnly) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.queue(SpawnCmdTaskCmd(f));
        self
    }
}

struct SpawnCmdTaskCmd<F>(F);

impl<F, Fut> EntityCommand for SpawnCmdTaskCmd<F>
where
    F: FnOnce(CmdOnly) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn apply(self, ewm: EntityWorldMut<'_>) {
        let entity = ewm.id();
        let world = ewm.into_world_mut();

        let queue = world.resource::<CmdQueue>().clone();
        let Some(rt_handle) = queue.handle.clone() else {
            return;
        };
        let token = CancellationToken::new();
        let cmd_only = CmdOnly { entity, queue };
        let fut = (self.0)(cmd_only);
        let child_token = token.child_token();

        let entity_for_complete = entity;
        let entity_for_abort = entity;

        let handle = {
            let queue_complete = world.resource::<CmdQueue>().clone();
            let queue_abort = world.resource::<CmdQueue>().clone();
            rt_handle.spawn(async move {
                tokio::select! {
                    _ = fut => {
                        queue_complete
                            .send(move |world: &mut World| {
                                world.trigger(TaskComplete(entity_for_complete));
                            })
                            .wake();
                    }
                    _ = child_token.cancelled() => {
                        queue_abort
                            .send(move |world: &mut World| {
                                world.trigger(TaskAborted(entity_for_abort));
                            })
                            .wake();
                    }
                }
            })
        };

        world.entity_mut(entity).insert(AsyncTask {
            token,
            _handle: handle,
        });
    }
}
