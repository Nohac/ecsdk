use bevy_ecs::prelude::*;

/// A closure that mutates the World. Sent through the channel by async tasks.
pub type WorldCmd = Box<dyn FnOnce(&mut World) + Send>;

/// Checked by main loop after each cycle to know when to exit.
#[derive(Resource, Default)]
pub struct AppExit(pub bool);
