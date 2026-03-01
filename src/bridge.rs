use bevy_ecs::prelude::*;
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedSender;

/// A closure that mutates the World. Sent through the channel by async tasks.
pub type WorldCmd = Box<dyn FnOnce(&mut World) + Send>;

/// Clone of the event channel sender. Systems use this to spawn async tasks
/// that send events back to the main loop.
#[derive(Resource, Clone)]
pub struct EventSender(pub UnboundedSender<WorldCmd>);

impl EventSender {
    /// Send a Bevy trigger event through the channel.
    pub fn trigger<E: Event + Send + 'static>(&self, event: E)
    where
        for<'a> E::Trigger<'a>: Default,
    {
        let _ = self.0.send(Box::new(move |world| {
            world.trigger(event);
            world.flush();
        }));
    }

    /// Send an arbitrary world mutation through the channel.
    pub fn send(&self, cmd: impl FnOnce(&mut World) + Send + 'static) {
        let _ = self.0.send(Box::new(cmd));
    }
}

/// Handle to the Tokio runtime. Needed because Bevy's multi-threaded schedule
/// runs systems on its own thread pool, which lacks Tokio context.
#[derive(Resource, Clone)]
pub struct TokioHandle(pub Handle);

/// Checked by main loop after each cycle to know when to exit.
#[derive(Resource, Default)]
pub struct AppExit(pub bool);
