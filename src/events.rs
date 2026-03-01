use bevy_ecs::prelude::*;

/// External event type flowing through the mpsc channel.
pub enum AppEvent {
    // Container lifecycle (sent by async tasks)
    DownloadProgress {
        entity: Entity,
        downloaded: u64,
        total: u64,
    },
    DownloadComplete(Entity),
    BootComplete(Entity),
    ShutdownComplete(Entity),

    // Internal
    ShutdownAll,
    AllContainersReady,
}

// ECS event/trigger types used inside the Bevy world.

#[derive(Event)]
pub struct DownloadCompleteEcs(pub Entity);

#[derive(Event)]
pub struct BootCompleteEcs(pub Entity);

#[derive(Event)]
pub struct ShutdownAllEcs;

#[derive(Event)]
pub struct ShutdownCompleteEcs(pub Entity);

/// Translates an external `AppEvent` into ECS world mutations / triggers.
pub fn inject_event(world: &mut World, event: AppEvent) {
    match event {
        AppEvent::DownloadProgress {
            entity,
            downloaded,
            total,
        } => {
            if let Some(mut progress) = world.get_mut::<crate::components::DownloadProgress>(entity)
            {
                progress.downloaded = downloaded;
                progress.total = total;
            }
        }
        AppEvent::DownloadComplete(entity) => {
            world.trigger(DownloadCompleteEcs(entity));
            world.flush();
        }
        AppEvent::BootComplete(entity) => {
            world.trigger(BootCompleteEcs(entity));
            world.flush();
        }
        AppEvent::ShutdownComplete(entity) => {
            world.trigger(ShutdownCompleteEcs(entity));
            world.flush();
        }
        // ShutdownAll and AllContainersReady are handled directly in the main loop
        _ => {}
    }
}
