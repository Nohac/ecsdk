use std::sync::Arc;

use bevy::ecs::prelude::*;
use tokio::sync::Notify;

/// Signals the main loop to run `app.update()` at the next fps boundary.
#[derive(Resource, Clone)]
pub struct TickSignal(pub Arc<Notify>);

/// Signals the main loop to run `app.update()` immediately.
#[derive(Resource, Clone)]
pub struct WakeSignal(pub Arc<Notify>);

/// Extension trait for requesting schedule updates.
pub trait ScheduleControl {
    fn tick(&mut self);
    fn wake(&mut self);
}

impl ScheduleControl for Commands<'_, '_> {
    fn tick(&mut self) {
        self.queue(|world: &mut World| world.tick());
    }

    fn wake(&mut self) {
        self.queue(|world: &mut World| world.wake());
    }
}

impl ScheduleControl for World {
    fn tick(&mut self) {
        self.resource::<TickSignal>().0.notify_one();
    }

    fn wake(&mut self) {
        self.resource::<WakeSignal>().0.notify_one();
    }
}
