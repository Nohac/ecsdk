use bevy::ecs::prelude::*;

/// Trait for domain message types that can mutate the world.
pub trait ApplyMessage: Send + 'static {
    fn apply(&self, world: &mut World);
}
