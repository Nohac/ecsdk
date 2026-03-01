use bevy_ecs::prelude::*;
use tokio::{
    select,
    signal::ctrl_c,
    sync::mpsc::{Receiver, channel, unbounded_channel},
};

#[derive(Component, Debug)]
struct Position {
    x: f32,
    y: f32,
}
#[derive(Component)]
struct Velocity {
    x: f32,
    y: f32,
}

#[derive(Component, Debug)]
struct VirtualMachine {
    name: String,
}

#[derive(Component, Debug)]
struct Stage {
    title: String,
    index: usize,
}

#[derive(Component, Debug)]
enum Status {
    Active,
    Skipped,
    Complete,
    Failed,
}

#[derive(Component, Debug)]
struct ProgressBar(usize);

#[derive(Component, Debug)]
struct ProgressStream(Vec<String>);

#[derive(Component, Debug)]
struct CommandWindow {
    w: usize,
    h: usize,
}

fn render_stream_stage(mut query: Query<(&Stage, &Status)>) {}

// impl<T> Stage<T> {
//     fn new(title: String, data: T) -> Self {
//
//     }
// }

// This system moves each entity with a Position and Velocity component
fn movement(mut query: Query<(&mut Position, &Velocity)>) {
    for (mut position, velocity) in &mut query {
        position.x += velocity.x;
        position.y += velocity.y;
        println!("{position:?}");
    }
}

#[derive(Event)]
struct SystemEvent;

fn initialize(mut c: Commands) {
    println!("sending an event");
    c.trigger(SystemEvent);
}

fn on_event_triggered(ev: On<SystemEvent>) {
    println!("event recieved");
}

enum EvTy {
    Start,
    Working,
    Stop,
}

#[tokio::main]
async fn main() {
    // Create a new empty World to hold our Entities and Components
    let mut world = World::new();

    // Spawn an entity with Position and Velocity components
    world.spawn((Position { x: 0.0, y: 0.0 }, Velocity { x: 1.0, y: 0.0 }));

    // Create a new Schedule, which defines an execution strategy for Systems
    let mut startup = Schedule::default();
    let mut update = Schedule::default();

    // Add our system to the schedule
    startup.add_systems(initialize);
    update.add_systems(movement);
    world.add_observer(on_event_triggered);

    startup.run(&mut world);
    let (tx, mut event) = unbounded_channel();
    // Run the schedule once. If your app has a "loop", you would run this once per loop
    vec![EvTy::Start, EvTy::Working, EvTy::Stop]
        .into_iter()
        .for_each(|e| tx.send(e).unwrap());

    loop {
        select! {
            ev = event.recv() => {
                // inject the event
                update.run(&mut world);
                continue;
            }
            _ = ctrl_c() => {break}
        }
    }
}
