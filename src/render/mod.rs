pub mod plain;

use bevy_ecs::prelude::*;

pub fn build_render_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.add_systems(plain::render_plain);
    schedule
}
