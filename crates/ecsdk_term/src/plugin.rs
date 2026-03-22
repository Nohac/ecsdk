use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use crossterm::event::{Event, EventStream};
use ecsdk_core::ScheduleControl;
use ecsdk_tasks::SpawnCmdTask;
use futures_util::StreamExt;

use crate::TerminalSize;

/// Terminal event forwarded into the ECS as a bevy Event.
#[derive(Event)]
pub struct TerminalEvent(pub Event);

/// Marker component for the crossterm event loop entity.
#[derive(Component)]
struct CrosstermEntity;

/// Plugin that sets up terminal size tracking and the crossterm event reader task.
pub struct TermPlugin;

impl Plugin for TermPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TerminalSize::query_now());
        app.add_systems(Startup, setup_crossterm);
    }
}

fn setup_crossterm(mut commands: Commands) {
    commands
        .spawn(CrosstermEntity)
        .spawn_cmd_task(|cmd| async move {
            let mut events = EventStream::new();
            while let Some(Ok(event)) = events.next().await {
                if let Event::Resize(cols, rows) = event {
                    cmd.send(move |world: &mut World| {
                        let mut size = world.resource_mut::<TerminalSize>();
                        size.cols = cols;
                        size.rows = rows;
                        world.tick();
                    });
                }
                let event_clone = event.clone();
                cmd.send(move |world: &mut World| {
                    world.trigger(TerminalEvent(event_clone));
                })
                .wake();
            }
        });
}
