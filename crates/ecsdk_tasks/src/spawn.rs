use std::future::Future;

use bevy::ecs::prelude::*;
use ecsdk_core::{ApplyMessage, CmdQueue, MessageQueue};
use tokio_util::sync::CancellationToken;

use crate::{AsyncTask, CmdOnly, TaskAborted, TaskComplete, TaskQueue};

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
        let cmd_only = CmdOnly::new(entity, queue);
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
