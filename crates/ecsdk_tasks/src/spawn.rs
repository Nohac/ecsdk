use std::future::Future;

use bevy::ecs::prelude::*;
use ecsdk_core::CmdQueue;
use tokio_util::sync::CancellationToken;

use crate::{AsyncTask, TaskAborted, TaskComplete, TaskQueue};

/// Extension trait for spawning entity-owned async tasks.
///
/// Spawned tasks are automatically cancelled when the owning entity is dropped,
/// and completion or cancellation is reported back into ECS as
/// [`TaskComplete`] and [`TaskAborted`] events.
pub trait SpawnTask {
    /// Spawns an async task owned by the target entity.
    fn spawn_task<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(TaskQueue) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static;
}

impl SpawnTask for EntityCommands<'_> {
    fn spawn_task<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(TaskQueue) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.queue(SpawnTaskCmd(f));
        self
    }
}

struct SpawnTaskCmd<F>(F);

impl<F, Fut> EntityCommand for SpawnTaskCmd<F>
where
    F: FnOnce(TaskQueue) -> Fut + Send + 'static,
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
        let task_queue = TaskQueue::new(entity, queue);
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
                        let _ = queue_complete.tx.send(Box::new(move |world: &mut World| {
                            world.trigger(TaskComplete(entity_for_complete));
                        }));
                    }
                    _ = child_token.cancelled() => {
                        let _ = queue_abort.tx.send(Box::new(move |world: &mut World| {
                            world.trigger(TaskAborted(entity_for_abort));
                        }));
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
