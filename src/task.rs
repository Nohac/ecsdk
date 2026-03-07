use std::future::Future;

use bevy::ecs::prelude::*;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::msg::{Queue, TaskQueue, TriggerEvent};

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

/// Extension trait for spawning entity-owned async tasks.
pub trait SpawnTask {
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

        let queue = world.resource::<Queue>().clone();
        let token = CancellationToken::new();
        let task_queue = TaskQueue::new(entity, queue.clone());
        let fut = (self.0)(task_queue);
        let child_token = token.child_token();

        let handle = {
            let handle = queue.handle.clone();
            handle.spawn(async move {
                tokio::select! {
                    _ = fut => {
                        queue.send(TriggerEvent(TaskComplete(entity)));
                    }
                    _ = child_token.cancelled() => {
                        queue.send(TriggerEvent(TaskAborted(entity)));
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
