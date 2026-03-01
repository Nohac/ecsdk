use bevy_ecs::prelude::*;
use tokio::{select, signal::ctrl_c, sync::mpsc};

use ecstest::components::*;
use ecstest::events::*;
use ecstest::render::build_render_schedule;
use ecstest::resources::{EventSender, TokioHandle};
use ecstest::systems::build_startup_schedule;
use ecstest::systems::build_update_schedule;
use ecstest::systems::lifecycle;

#[tokio::main]
async fn main() {
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let tx_ctrl_c = tx.clone();

    let mut world = World::new();
    world.insert_resource(EventSender(tx));
    world.insert_resource(TokioHandle(tokio::runtime::Handle::current()));

    // Register observers
    world.add_observer(lifecycle::handle_download_complete);
    world.add_observer(lifecycle::handle_boot_complete);
    world.add_observer(lifecycle::handle_shutdown_all);
    world.add_observer(lifecycle::handle_shutdown_complete);

    let mut startup = build_startup_schedule();
    let mut update = build_update_schedule();
    let mut render = build_render_schedule();

    // Startup: spawn containers, then run ordering (kicks off first downloads)
    startup.run(&mut world);
    update.run(&mut world);
    render.run(&mut world);

    let mut shutting_down = false;

    // Event loop — purely reactive, only wakes on events
    loop {
        select! {
            event = rx.recv() => {
                let Some(event) = event else { break };
                match event {
                    AppEvent::AllContainersReady => {
                        render.run(&mut world);
                        println!("\nAll containers ready.");
                        break;
                    }
                    AppEvent::ShutdownAll => {
                        if !shutting_down {
                            shutting_down = true;
                            println!("\nShutting down...");
                            world.trigger(ShutdownAllEcs);
                            world.flush();
                            update.run(&mut world);
                            render.run(&mut world);
                        }
                    }
                    AppEvent::ShutdownComplete(entity) => {
                        inject_event(&mut world, AppEvent::ShutdownComplete(entity));
                        update.run(&mut world);
                        render.run(&mut world);

                        // Check if all containers are now stopped
                        let all_stopped = world
                            .query::<&ContainerPhase>()
                            .iter(&world)
                            .all(|p| *p == ContainerPhase::Stopped);
                        if all_stopped && shutting_down {
                            println!("\nAll containers stopped.");
                            break;
                        }
                    }
                    event => {
                        inject_event(&mut world, event);
                        update.run(&mut world);
                        render.run(&mut world);
                    }
                }
            }
            _ = ctrl_c() => {
                let _ = tx_ctrl_c.send(AppEvent::ShutdownAll);
            }
        }
    }
}
