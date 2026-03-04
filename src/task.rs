use std::future::Future;

use bevy_ecs::prelude::*;
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::bridge::WorldCmd;

/// Resource that bridges async tasks to the ECS world.
/// Carries both the command channel sender and the Tokio runtime handle.
#[derive(Resource, Clone)]
pub struct CommandSender {
    tx: UnboundedSender<WorldCmd>,
    handle: Handle,
}

impl CommandSender {
    pub fn new(tx: UnboundedSender<WorldCmd>, handle: Handle) -> Self {
        Self { tx, handle }
    }

    /// Send a Bevy trigger event through the channel.
    pub fn trigger<E: Event + Send + 'static>(&self, event: E)
    where
        for<'a> E::Trigger<'a>: Default,
    {
        let _ = self.tx.send(Box::new(move |world| {
            world.trigger(event);
            world.flush();
        }));
    }

    /// Send an arbitrary world mutation through the channel.
    pub fn send(&self, cmd: impl FnOnce(&mut World) + Send + 'static) {
        let _ = self.tx.send(Box::new(cmd));
    }

    /// Run a closure on the world and return its result.
    pub async fn query<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut World) -> T + Send + 'static,
    ) -> T {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.send(move |world| {
            let _ = tx.send(f(world));
        });
        rx.await.expect("world loop dropped before responding")
    }
}

/// Handle passed to async task closures. Provides access to the owning entity
/// and a way to push world mutations back to the ECS.
#[derive(Clone)]
pub struct TaskCommands {
    entity: Entity,
    tx: CommandSender,
}

impl TaskCommands {
    pub fn entity(&self) -> Entity {
        self.entity
    }

    /// Push a world mutation to be applied on the next tick.
    pub fn push(&self, cmd: impl FnOnce(&mut World) + Send + 'static) {
        self.tx.send(cmd);
    }

    /// Trigger a Bevy event through the command channel.
    pub fn trigger<E: Event + Send + 'static>(&self, event: E)
    where
        for<'a> E::Trigger<'a>: Default,
    {
        self.tx.trigger(event);
    }
}

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
        F: FnOnce(TaskCommands) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static;
}

impl SpawnTask for EntityCommands<'_> {
    fn spawn_task<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(TaskCommands) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.queue(SpawnTaskCmd(f));
        self
    }
}

struct SpawnTaskCmd<F>(F);

impl<F, Fut> EntityCommand for SpawnTaskCmd<F>
where
    F: FnOnce(TaskCommands) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn apply(self, ewm: EntityWorldMut<'_>) {
        let entity = ewm.id();
        let world = ewm.into_world_mut();

        let cmd_sender = world.resource::<CommandSender>().clone();
        let token = CancellationToken::new();
        let task_cmd = TaskCommands {
            entity,
            tx: cmd_sender.clone(),
        };
        let fut = (self.0)(task_cmd);
        let child_token = token.child_token();

        let handle = {
            let handle = cmd_sender.handle.clone();
            handle.spawn(async move {
                tokio::select! {
                    _ = fut => {
                        cmd_sender.trigger(TaskComplete(entity));
                    }
                    _ = child_token.cancelled() => {
                        cmd_sender.trigger(TaskAborted(entity));
                    }
                }
            })
        };

        world
            .entity_mut(entity)
            .insert(AsyncTask { token, _handle: handle });
    }
}
