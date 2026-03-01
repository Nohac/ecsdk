use bevy_ecs::prelude::*;
use tokio::{select, signal::ctrl_c, sync::mpsc};

use ecstest::bridge::{AppExit, EventSender, TokioHandle, WorldCmd};
use ecstest::container::build_startup_schedule;
use ecstest::lifecycle::{ShutdownAll, build_update_schedule, register_observers};
use ecstest::render::build_render_schedule;

#[tokio::main]
async fn main() {
    let (tx, mut rx) = mpsc::unbounded_channel::<WorldCmd>();
    let tx_ctrl_c = EventSender(tx.clone());

    let mut world = World::new();
    world.insert_resource(EventSender(tx));
    world.insert_resource(TokioHandle(tokio::runtime::Handle::current()));
    world.init_resource::<AppExit>();

    register_observers(&mut world);

    let mut startup = build_startup_schedule();
    let mut update = build_update_schedule();
    let mut render = build_render_schedule();

    startup.run(&mut world);
    update.run(&mut world);
    render.run(&mut world);

    loop {
        select! {
            Some(cmd) = rx.recv() => {
                cmd(&mut world);
                update.run(&mut world);
                render.run(&mut world);
                if world.resource::<AppExit>().0 { break; }
            }
            _ = ctrl_c() => {
                tx_ctrl_c.trigger(ShutdownAll);
            }
        }
    }
}
